//! Runtime glue: conecta `oxi-net` ↔ `oxi-engine` ↔ interfaz con canales de
//! tokio (DESIGN §6). El engine sigue siendo puro: este módulo materializa
//! sus acciones (enviar, mostrar, arrancar timers) y le inyecta eventos.

use std::collections::HashMap;
use std::time::Duration;

use oxi_core::{parse_host, HostMessage, OperationalMode, ParseOptions, ScreenSelector};
use oxi_engine::{Action, Event, InputEcho, Terminal};
use oxi_net::{connect_with_backoff, pump, ConnectionState, ReconnectConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::config::SimConfig;
use crate::screen::render_screen;

/// Actualizaciones para la interfaz (CLI v1; GUI en fase 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiUpdate {
    /// Línea de log del motor o del runtime.
    Log(String),
    /// Pantalla mostrada (o limpiada, con `None`) y su texto renderizado.
    Screen {
        /// Identificador de la pantalla.
        id: Option<ScreenSelector>,
        /// Texto legible (render aproximado).
        text: String,
    },
    /// Cambio de modo operativo.
    Mode(OperationalMode),
    /// Cambio de estado de la conexión.
    Connection(ConnectionState),
    /// Eco local del campo de entrada (`None` = sin captura con eco).
    InputEcho(Option<InputEcho>),
    /// Efectivo dispensado pendiente de retiro (para el botón de la GUI).
    CashReady {
        /// Billetes por casete.
        counts: [u16; 4],
    },
    /// La sesión de red terminó (fallo de conexión o enlace cerrado).
    SessionEnded,
}

/// Handle de una sesión de oxi en ejecución.
pub struct RuntimeHandle {
    /// Stream de actualizaciones para la interfaz.
    pub updates: mpsc::Receiver<UiUpdate>,
    events: mpsc::Sender<Event>,
    tasks: Vec<JoinHandle<()>>,
}

impl RuntimeHandle {
    /// Inyecta un evento (tecla, tarjeta, etc.). `false` = runtime cerrado.
    pub async fn send(&self, event: Event) -> bool {
        self.events.send(event).await.is_ok()
    }

    /// Variante sin bloqueo para interfaces sincrónicas como egui.
    /// `false` = canal lleno o runtime cerrado (el evento se descarta).
    pub fn try_send(&self, event: Event) -> bool {
        self.events.try_send(event).is_ok()
    }

    /// Corta la sesión (red + motor).
    pub fn shutdown(self) {
        for task in self.tasks {
            task.abort();
        }
    }
}

/// Arranca una sesión: red + motor + planificador de timers.
pub fn spawn(config: &SimConfig) -> RuntimeHandle {
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(64);
    let (frame_in_tx, mut frame_in_rx) = mpsc::channel::<Vec<u8>>(64);
    let (frame_out_tx, frame_out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (update_tx, update_rx) = mpsc::channel::<UiUpdate>(256);

    // --- tarea de red ---
    let addr = config.host_addr();
    let max_attempts = config.net.max_attempts;
    let net_updates = update_tx.clone();
    let net_task = tokio::spawn(async move {
        let (state_tx, mut state_rx) = mpsc::channel::<ConnectionState>(16);
        let watcher_updates = net_updates.clone();
        let watcher = tokio::spawn(async move {
            while let Some(state) = state_rx.recv().await {
                if watcher_updates
                    .send(UiUpdate::Connection(state))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        let reconnect = ReconnectConfig {
            max_attempts,
            ..ReconnectConfig::default()
        };
        let stream = match connect_with_backoff(addr.as_str(), &reconnect, &state_tx).await {
            Ok(stream) => stream,
            Err(err) => {
                let _ = net_updates
                    .send(UiUpdate::Log(format!("conexión fallida: {err}")))
                    .await;
                let _ = net_updates.send(UiUpdate::SessionEnded).await;
                watcher.abort();
                return;
            }
        };
        match pump(stream, frame_in_tx, frame_out_rx).await {
            Ok(end) => {
                let _ = net_updates
                    .send(UiUpdate::Log(format!("conexión terminada: {end:?}")))
                    .await;
            }
            Err(err) => {
                let _ = net_updates
                    .send(UiUpdate::Log(format!("error de transporte: {err}")))
                    .await;
            }
        }
        let _ = net_updates.send(UiUpdate::SessionEnded).await;
        watcher.abort();
    });

    // --- tarea del motor ---
    let engine_updates = update_tx;
    let engine_events = event_tx.clone();
    let security = config.security.to_engine();
    let engine_task = tokio::spawn(async move {
        let mut terminal = Terminal::new();
        terminal.configure_security(security);
        let mut timers: HashMap<u8, JoinHandle<()>> = HashMap::new();
        let options = ParseOptions::default();
        let mut last_echo: Option<InputEcho> = None;
        let initial = terminal.handle(Event::PoweredOn);
        dispatch(
            &mut terminal,
            initial,
            &engine_updates,
            &frame_out_tx,
            &engine_events,
            &mut timers,
            &mut last_echo,
        )
        .await;

        loop {
            let actions = tokio::select! {
                // Los mensajes del host tienen prioridad sobre los eventos de
                // la interfaz cuando ambos están listos (menos ventana de
                // carrera entre un comando y una tecla).
                biased;
                Some(frame) = frame_in_rx.recv() => {
                    match parse_host(&oxi_core::RawFields::parse(&frame), &options) {
                        Ok(message) => {
                            terminal.handle(Event::HostMessage(message))
                        }
                        Err(err) => {
                            let reject = err.reject_code().to_owned();
                            let _ = engine_updates.send(UiUpdate::Log(format!(
                                "mensaje del host inválido: {err} (reject {reject})"
                            ))).await;
                            terminal.handle(Event::HostParseError { reject })
                        }
                    }
                }
                Some(event) = event_rx.recv() => terminal.handle(event),
                else => break,
            };
            dispatch(
                &mut terminal,
                actions,
                &engine_updates,
                &frame_out_tx,
                &engine_events,
                &mut timers,
                &mut last_echo,
            )
            .await;
        }
    });

    RuntimeHandle {
        updates: update_rx,
        events: event_tx,
        tasks: vec![net_task, engine_task],
    }
}

/// Ejecuta las acciones del motor contra los canales y el planificador.
async fn dispatch(
    terminal: &mut Terminal,
    actions: Vec<Action>,
    updates: &mpsc::Sender<UiUpdate>,
    frames_out: &mpsc::Sender<Vec<u8>>,
    events: &mpsc::Sender<Event>,
    timers: &mut HashMap<u8, JoinHandle<()>>,
    last_echo: &mut Option<InputEcho>,
) {
    for action in actions {
        match action {
            Action::Send(message) => match message.to_bytes() {
                Ok(bytes) => {
                    let _ = frames_out.send(bytes).await;
                }
                Err(err) => {
                    let _ = updates
                        .send(UiUpdate::Log(format!("mensaje no serializable: {err}")))
                        .await;
                }
            },
            Action::Display(id) => {
                let text = id
                    .as_ref()
                    .and_then(|id| terminal.screen_content(id))
                    .map(render_screen)
                    .unwrap_or_default();
                let _ = updates.send(UiUpdate::Screen { id, text }).await;
            }
            Action::DisplayData(data) => {
                let text = render_screen(&data);
                let _ = updates.send(UiUpdate::Screen { id: None, text }).await;
            }
            Action::StartTimer { timer, after } => {
                if let Some(previous) = timers.remove(&timer) {
                    previous.abort();
                }
                if let Some(after) = after {
                    let timer_events = events.clone();
                    timers.insert(
                        timer,
                        tokio::spawn(async move {
                            tokio::time::sleep(after).await;
                            let _ = timer_events.send(Event::TimerFired(timer)).await;
                        }),
                    );
                }
            }
            Action::CancelTimer(timer) => {
                if let Some(task) = timers.remove(&timer) {
                    task.abort();
                }
            }
            Action::Log(line) => {
                let _ = updates.send(UiUpdate::Log(line)).await;
            }
            Action::ModeChanged(mode) => {
                let _ = updates.send(UiUpdate::Mode(mode)).await;
            }
            Action::Print { printer, data } => {
                let _ = updates
                    .send(UiUpdate::Log(format!("[print {printer:?}] {data}")))
                    .await;
            }
            Action::EjectCard => {
                let _ = updates
                    .send(UiUpdate::Log("tarjeta devuelta al cardholder".into()))
                    .await;
            }
            Action::Dispense(notes) => {
                let counts = notes.counts();
                let _ = updates
                    .send(UiUpdate::Log(format!("dispensando {counts:?} billetes")))
                    .await;
                // Evento tipado para que la GUI habilite "Retirar efectivo".
                let _ = updates.send(UiUpdate::CashReady { counts }).await;
            }
        }
    }

    // Eco local de la captura en curso (PIN/importe/info). Se manda solo al
    // cambiar; el motor decide qué mostrar (manual §2-8, §2-17, §2-23).
    let echo = terminal.input_echo();
    if echo != *last_echo {
        let _ = updates.send(UiUpdate::InputEcho(echo.clone())).await;
        *last_echo = echo;
    }
}

/// Evento sintético de tarjeta a partir del perfil.
pub fn card_event(profile: &crate::config::CardProfile) -> Event {
    Event::CardInserted(oxi_engine::Card {
        track2: Some(profile.track2.clone()),
        ..oxi_engine::Card::default()
    })
}

/// Convierte un mensaje del host en evento (helper para tests).
pub fn host_event(message: HostMessage) -> Event {
    Event::HostMessage(message)
}

/// Duración por defecto para esperas de la CLI/tests.
pub const WAIT: Duration = Duration::from_secs(5);
