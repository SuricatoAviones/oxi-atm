//! Conexión TCP del ATM hacia el host, con reintentos de backoff
//! exponencial (SPEC §1; DESIGN §4). El ATM es cliente: inicia la conexión.

use std::time::Duration;

use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::sync::mpsc;

use crate::error::NetError;

/// Estado de la conexión, observable por la UI/runtime (DESIGN §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Intentando conectar (número de intento, desde 1).
    Connecting {
        /// Intento en curso.
        attempt: u32,
    },
    /// Conexión establecida.
    Connected,
}

/// Configuración de reintentos de conexión.
#[derive(Debug, Clone, Copy)]
pub struct ReconnectConfig {
    /// Intentos máximos antes de devolver error.
    pub max_attempts: u32,
    /// Backoff del primer reintento.
    pub initial_backoff: Duration,
    /// Tope del backoff exponencial.
    pub max_backoff: Duration,
}

impl Default for ReconnectConfig {
    /// 8 intentos, 1 s → 60 s de backoff: pragmático para un simulador que
    /// espera a que el host levante.
    fn default() -> Self {
        Self {
            max_attempts: 8,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

impl ReconnectConfig {
    /// Backoff del intento `attempt` (exponencial con tope).
    fn backoff_for(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1);
        let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        self.initial_backoff
            .saturating_mul(factor as u32)
            .min(self.max_backoff)
    }
}

/// Conecta al host con reintentos de backoff exponencial.
///
/// Reporta cada intento por `state_tx` (la UI lo muestra; el runtime lo
/// loguea). Devuelve el stream conectado o el último error de I/O.
pub async fn connect_with_backoff<A: ToSocketAddrs>(
    addr: A,
    config: &ReconnectConfig,
    state_tx: &mpsc::Sender<ConnectionState>,
) -> Result<TcpStream, NetError> {
    let mut last_err = None;
    for attempt in 1..=config.max_attempts {
        let _ = state_tx.send(ConnectionState::Connecting { attempt }).await;
        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                let _ = state_tx.send(ConnectionState::Connected).await;
                return Ok(stream);
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < config.max_attempts {
                    tokio::time::sleep(config.backoff_for(attempt)).await;
                }
            }
        }
    }
    Err(NetError::Io(last_err.unwrap_or_else(|| {
        std::io::Error::other("sin intentos de conexión")
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    /// Cierra el sender y drena los estados con timeout: un deadlock de
    /// canales falla rápido en CI en vez de colgar el runner.
    async fn drain_states(
        mut rx: mpsc::Receiver<ConnectionState>,
        tx: mpsc::Sender<ConnectionState>,
    ) -> Vec<ConnectionState> {
        drop(tx);
        tokio::time::timeout(StdDuration::from_secs(5), async move {
            let mut states = Vec::new();
            while let Some(state) = rx.recv().await {
                states.push(state);
            }
            states
        })
        .await
        .expect("el drenaje de estados no debe colgarse")
    }

    /// Backoff exponencial con tope: 1s, 2s, 4s… hasta 60s.
    #[test]
    fn backoff_should_double_until_cap() {
        let config = ReconnectConfig {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            ..ReconnectConfig::default()
        };
        assert_eq!(config.backoff_for(1), Duration::from_secs(1));
        assert_eq!(config.backoff_for(2), Duration::from_secs(2));
        assert_eq!(config.backoff_for(4), Duration::from_secs(8));
        assert_eq!(config.backoff_for(7), Duration::from_secs(60));
    }

    /// Conexión feliz a un servidor de juguete: un intento, Connected.
    #[tokio::test]
    async fn connect_should_succeed_on_first_attempt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("puerto efímero");
        let addr = listener.local_addr().expect("addr");

        let (state_tx, mut state_rx) = mpsc::channel(8);
        let config = ReconnectConfig {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
        };
        let stream = connect_with_backoff(addr, &config, &state_tx)
            .await
            .expect("el listener está arriba");
        drop(stream);

        let first = state_rx.recv().await.expect("estado reportado");
        assert_eq!(first, ConnectionState::Connecting { attempt: 1 });
        assert_eq!(state_rx.recv().await, Some(ConnectionState::Connected));
    }

    /// Sin servidor: agota los intentos con backoff corto y devuelve el
    /// error de I/O. (Se usa un puerto tomado y liberado para asegurar el
    /// rechazo de conexión.)
    #[tokio::test]
    async fn connect_should_give_up_after_max_attempts() {
        // Puerto ocupado y liberado: el siguiente connect debe ser rechazado.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("puerto efímero");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let (state_tx, state_rx) = mpsc::channel(16);
        let config = ReconnectConfig {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(10),
        };
        let result = connect_with_backoff(addr, &config, &state_tx).await;
        assert!(result.is_err(), "sin listener debe fallar");

        let states = drain_states(state_rx, state_tx).await;
        let attempts = states
            .iter()
            .filter(|s| matches!(s, ConnectionState::Connecting { .. }))
            .count();
        assert_eq!(attempts, 3, "debe agotar exactamente max_attempts");
    }

    /// El host aparece en el segundo intento: reconecta y reporta Connected.
    #[tokio::test]
    async fn connect_should_recover_when_host_comes_up() {
        // Primer intento a un puerto muerto.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("puerto efímero");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let (state_tx, state_rx) = mpsc::channel(16);
        let config = ReconnectConfig {
            max_attempts: 10,
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(50),
        };

        // El host "levanta" mientras el primer intento duerme su backoff.
        let relay = addr;
        let host_up = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            tokio::net::TcpListener::bind(relay)
                .await
                .expect("el host levanta")
        });

        let stream = connect_with_backoff(addr, &config, &state_tx)
            .await
            .expect("conecta al segundo intento");
        drop(stream);
        let listener = host_up.await.expect("task del host");
        drop(listener);

        // Estados: Connecting(1)… fallo… Connecting(2)… Connected.
        let states = drain_states(state_rx, state_tx).await;
        let saw_second_attempt = states
            .iter()
            .any(|s| matches!(s, ConnectionState::Connecting { attempt } if *attempt >= 2));
        let saw_connected = states.contains(&ConnectionState::Connected);
        assert!(saw_second_attempt, "debe reintentar tras el primer fallo");
        assert!(saw_connected);
    }
}
