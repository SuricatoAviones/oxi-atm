//! Host falso guionizado: acepta la conexión del ATM, baja la customización
//! canónica, lo pone en servicio y responde transacciones.
//!
//! Roles (SPEC §1): el ATM es el cliente TCP; el host escucha.

use std::net::SocketAddr;
use std::time::Duration;

use oxi_core::{parse_terminal, HostMessage, RawFields, StatusDescriptor, TerminalMessage};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::fixtures;
use crate::framing::{read_frame, write_frame};

/// Error del host falso.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Error de I/O (conexión, cierre).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// El ATM envió algo que no parsea.
    #[error("mensaje del ATM inválido: {0}")]
    Parse(#[from] oxi_core::ParseError),
    /// Encuadre inválido.
    #[error("encuadre: {0}")]
    Framing(String),
    /// El guion esperaba otro mensaje.
    #[error("guion: {0}")]
    Script(String),
}

/// Configuración del guion.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Aprobar (true) o rechazar (false) los Transaction Requests.
    pub approve: bool,
    /// Silencio máximo esperando al ATM antes de cerrar la sesión.
    pub idle_timeout: Duration,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            approve: true,
            idle_timeout: Duration::from_secs(2),
        }
    }
}

/// Resumen de la sesión (para tests y CLI).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostReport {
    /// Frames recibidos del ATM.
    pub frames_in: usize,
    /// Frames enviados al ATM.
    pub frames_out: usize,
    /// Transaction Requests atendidos.
    pub transactions: usize,
    /// Descriptores de status recibidos (en orden).
    pub statuses: Vec<String>,
}

/// Host falso escuchando en un puerto.
pub struct ScriptedHost {
    listener: TcpListener,
}

impl ScriptedHost {
    /// Escucha en la dirección dada (p.ej. `127.0.0.1:0` en tests).
    pub async fn bind(addr: &str) -> Result<Self, HostError> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }

    /// Dirección efectiva (útil con puerto 0).
    pub fn local_addr(&self) -> Result<SocketAddr, HostError> {
        Ok(self.listener.local_addr()?)
    }

    /// Atiende UNA conexión de ATM con el guion completo.
    ///
    /// 1. Acepta la conexión.
    /// 2. Baja la customización canónica (cada mensaje espera Ready).
    /// 3. Manda Go In-Service (espera Ready).
    /// 4. Pide el Config ID (espera Terminal State 'F').
    /// 5. Responde TRQ (aprobar/rechazar) hasta el silencio del ATM.
    pub async fn run_once(&self, config: &HostConfig) -> Result<HostReport, HostError> {
        let (mut stream, _peer) = self.listener.accept().await?;
        let mut report = HostReport::default();

        // Descarga de customización.
        for message in fixtures::customisation() {
            send_host(&mut stream, &message, &mut report).await?;
            let response = read_terminal(&mut stream, config.idle_timeout, &mut report).await?;
            expect_ready(&response, &mut report)?;
        }

        // A servicio.
        send_host(&mut stream, &fixtures::go_in_service(), &mut report).await?;
        let response = read_terminal(&mut stream, config.idle_timeout, &mut report).await?;
        expect_ready(&response, &mut report)?;

        // Petición del Config ID (comando 3) y su respuesta Terminal State.
        // Un host real atiende TRQs en cualquier momento: si aparece uno
        // mientras se espera el Terminal State, se responde y se sigue.
        send_host(&mut stream, &fixtures::request_config_id(), &mut report).await?;
        loop {
            match read_terminal(&mut stream, config.idle_timeout, &mut report).await? {
                TerminalMessage::SolicitedStatus {
                    descriptor: StatusDescriptor::TerminalState,
                    ..
                } => break,
                TerminalMessage::TransactionRequest(request) => {
                    report.transactions += 1;
                    let reply = fixtures::transaction_reply(&request, config.approve);
                    send_host(&mut stream, &reply, &mut report).await?;
                }
                _ => {}
            }
        }

        // Bucle de transacciones hasta el silencio del ATM.
        loop {
            match read_terminal(&mut stream, config.idle_timeout, &mut report).await {
                Ok(TerminalMessage::TransactionRequest(request)) => {
                    report.transactions += 1;
                    let reply = fixtures::transaction_reply(&request, config.approve);
                    send_host(&mut stream, &reply, &mut report).await?;
                }
                Ok(_) => {}
                Err(HostError::Io(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof
                        || err.kind() == std::io::ErrorKind::ConnectionReset =>
                {
                    return Ok(report);
                }
                // Silencio del ATM (idle timeout): sesión terminada.
                Err(HostError::Io(err)) if err.kind() == std::io::ErrorKind::TimedOut => {
                    return Ok(report);
                }
                Err(err) => return Err(err),
            }
        }
    }
}

async fn send_host(
    stream: &mut TcpStream,
    message: &HostMessage,
    report: &mut HostReport,
) -> Result<(), HostError> {
    let bytes = message.to_bytes()?;
    write_frame(stream, &bytes).await?;
    report.frames_out += 1;
    Ok(())
}

async fn read_terminal(
    stream: &mut TcpStream,
    idle_timeout: Duration,
    report: &mut HostReport,
) -> Result<TerminalMessage, HostError> {
    let frame = match timeout(idle_timeout, read_frame(stream)).await {
        Ok(result) => result?,
        Err(_) => {
            return Err(HostError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "sin datos del ATM",
            )))
        }
    };
    let Some(frame) = frame else {
        return Err(HostError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "el ATM cerró la conexión",
        )));
    };
    report.frames_in += 1;
    let message = parse_terminal(&RawFields::parse(&frame))?;
    if let TerminalMessage::SolicitedStatus { descriptor, .. } = &message {
        report.statuses.push(format!("{descriptor:?}"));
    }
    Ok(message)
}

fn expect_ready(message: &TerminalMessage, report: &mut HostReport) -> Result<(), HostError> {
    match message {
        TerminalMessage::SolicitedStatus {
            descriptor: StatusDescriptor::Ready | StatusDescriptor::ReadyAlternate,
            ..
        } => Ok(()),
        other => Err(HostError::Script(format!(
            "se esperaba Ready, llegó {other:?} (statuses: {:?})",
            report.statuses
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El guion completo contra un ATM de prueba que responde Ready:
    /// descarga (6 mensajes + in-service + config id) y una transacción.
    #[tokio::test]
    async fn script_should_download_and_handle_one_transaction() {
        let host = ScriptedHost::bind("127.0.0.1:0").await.expect("bind");
        let addr = host.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            host.run_once(&HostConfig {
                approve: true,
                idle_timeout: Duration::from_millis(500),
            })
            .await
        });

        let mut stream = TcpStream::connect(addr).await.expect("conecta");
        // Un ATM mínimo: responde Ready a los loads/in-service y Terminal
        // State al pedido de Config ID (comando 3).
        let options = oxi_core::ParseOptions::default();
        let mut expected = fixtures::customisation();
        expected.push(fixtures::go_in_service());
        expected.push(fixtures::request_config_id());
        for _ in expected {
            let frame = read_frame(&mut stream)
                .await
                .expect("frame del host")
                .expect("no EOF");
            let message =
                oxi_core::parse_host(&RawFields::parse(&frame), &options).expect("host válido");
            let response = match &message {
                HostMessage::TerminalCommand(oxi_core::TerminalCommand::SendConfigurationId) => {
                    TerminalMessage::SolicitedStatus {
                        luno: oxi_core::Luno::new("001").expect("válido"),
                        descriptor: StatusDescriptor::TerminalState,
                        data: vec!["60001".into()],
                    }
                }
                _ => TerminalMessage::SolicitedStatus {
                    luno: oxi_core::Luno::new("001").expect("válido"),
                    descriptor: StatusDescriptor::Ready,
                    data: Vec::new(),
                },
            };
            write_frame(&mut stream, &response.to_bytes().expect("respuesta"))
                .await
                .expect("escribe");
        }

        // Una transacción: el ATM manda un TRQ y espera el reply.
        let request = TerminalMessage::TransactionRequest(Box::new(oxi_core::TransactionRequest {
            luno: oxi_core::Luno::new("001").expect("válido"),
            tvn: None,
            top_of_receipt: '1',
            mcn: oxi_core::Mcn::new('\u{31}', false).expect("válido"),
            track2: Some(fixtures::test_card()),
            track3: None,
            op_code: None,
            amount: Some("00000100".into()),
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: None,
            status_data: None,
        }));
        write_frame(&mut stream, &request.to_bytes().expect("trq"))
            .await
            .expect("escribe");

        let frame = read_frame(&mut stream)
            .await
            .expect("reply del host")
            .expect("no EOF");
        let reply =
            oxi_core::parse_host(&RawFields::parse(&frame), &options).expect("reply válido");
        let HostMessage::TransactionReply(reply) = reply else {
            panic!("se esperaba Transaction Reply, llegó {reply:?}");
        };
        assert_eq!(reply.mcn, Some('\u{31}'), "el host ecoa el MCN");
        assert!(reply.notes.is_some(), "aprobado con dispensado");

        drop(stream);
        let report = server.await.expect("join").expect("guion ok");
        assert_eq!(report.transactions, 1);
        assert_eq!(
            report.frames_out, 9,
            "6 loads + in-service + comando 3 + reply"
        );
    }
}
