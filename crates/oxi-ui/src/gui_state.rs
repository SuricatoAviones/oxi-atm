//! Estado observable de la interfaz (fase 9), independiente de egui.
//!
//! La GUI consume [`UiUpdate`] en cada frame y lo aplica acá; así el
//! view-model se puede testear sin abrir una ventana (y la CLI sigue usando
//! los mismos eventos).

use std::collections::VecDeque;

use oxi_core::{OperationalMode, ScreenSelector};
use oxi_engine::InputEcho;
use oxi_net::ConnectionState;

use crate::runtime::UiUpdate;

/// Máximo de líneas de log que conserva la GUI (memoria acotada).
pub const MAX_LOG_LINES: usize = 400;

/// Estado que la ventana dibuja.
#[derive(Debug, Default)]
pub struct GuiState {
    /// Pantalla actual (None = pantalla limpia).
    pub screen_id: Option<ScreenSelector>,
    /// Texto renderizado de la pantalla actual.
    pub screen_text: String,
    /// Último modo operativo informado por el motor.
    pub mode: Option<OperationalMode>,
    /// Último estado de conexión informado por la red.
    pub connection: Option<ConnectionState>,
    /// Líneas de log (las más viejas se descartan).
    pub logs: VecDeque<String>,
    /// Eco local del campo de entrada (PIN/importe), si hay captura.
    pub input_echo: Option<InputEcho>,
    /// Efectivo dispensado pendiente de retiro (botón de la GUI).
    pub cash_ready: Option<[u16; 4]>,
    /// El runtime cerró (red o motor terminaron).
    pub session_closed: bool,
}

impl GuiState {
    /// Aplica una actualización del runtime.
    pub fn apply(&mut self, update: UiUpdate) {
        match update {
            UiUpdate::Log(line) => self.push_log(line),
            UiUpdate::Screen { id, text } => {
                self.screen_id = id;
                self.screen_text = text;
            }
            UiUpdate::Mode(mode) => self.mode = Some(mode),
            UiUpdate::Connection(state) => self.connection = Some(state),
            UiUpdate::InputEcho(echo) => self.input_echo = echo,
            UiUpdate::CashReady { counts } => self.cash_ready = Some(counts),
            UiUpdate::SessionEnded => self.mark_closed(),
        }
    }

    /// Agrega una línea de log respetando el tope.
    pub fn push_log(&mut self, line: impl Into<String>) {
        if self.logs.len() == MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(line.into());
    }

    /// Vacía el panel de logs (botón "Limpiar" de la GUI).
    pub fn clear_logs(&mut self) {
        self.logs.clear();
    }

    /// Marca la sesión como terminada una sola vez (evita repetir el aviso).
    pub fn mark_closed(&mut self) {
        if !self.session_closed {
            self.session_closed = true;
            self.push_log("sesión terminada (runtime cerrado)");
        }
    }

    /// El cardholder retiró el efectivo: ya no hay nada pendiente.
    pub fn cash_taken(&mut self) {
        self.cash_ready = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxi_core::ScreenId;

    #[test]
    fn screen_update_should_replace_text_and_id() {
        let mut state = GuiState::default();
        state.apply(UiUpdate::Screen {
            id: Some(ScreenSelector::Customer(
                ScreenId::new(12).expect("pantalla válida"),
            )),
            text: "BIENVENIDO".into(),
        });
        assert_eq!(state.screen_text, "BIENVENIDO");
        assert!(state.screen_id.is_some());
    }

    #[test]
    fn logs_should_be_capped_at_max() {
        let mut state = GuiState::default();
        for index in 0..(MAX_LOG_LINES + 3) {
            state.push_log(format!("línea {index}"));
        }
        assert_eq!(state.logs.len(), MAX_LOG_LINES);
        assert_eq!(state.logs.front().map(String::as_str), Some("línea 3"));
    }

    #[test]
    fn mode_and_connection_should_track_the_last_update() {
        let mut state = GuiState::default();
        state.apply(UiUpdate::Mode(OperationalMode::OutOfService));
        state.apply(UiUpdate::Mode(OperationalMode::InService));
        state.apply(UiUpdate::Connection(ConnectionState::Connecting {
            attempt: 2,
        }));
        assert_eq!(state.mode, Some(OperationalMode::InService));
        assert_eq!(
            state.connection,
            Some(ConnectionState::Connecting { attempt: 2 })
        );
    }

    #[test]
    fn cash_ready_should_clear_when_taken() {
        let mut state = GuiState::default();
        state.apply(UiUpdate::CashReady {
            counts: [1, 0, 0, 0],
        });
        assert_eq!(state.cash_ready, Some([1, 0, 0, 0]));
        state.cash_taken();
        assert_eq!(state.cash_ready, None);
    }

    #[test]
    fn input_echo_should_track_masked_and_clear() {
        let mut state = GuiState::default();
        state.apply(UiUpdate::InputEcho(Some(InputEcho {
            text: "XXXX".into(),
            masked: true,
        })));
        assert_eq!(
            state.input_echo,
            Some(InputEcho {
                text: "XXXX".into(),
                masked: true,
            })
        );
        state.apply(UiUpdate::InputEcho(None));
        assert_eq!(state.input_echo, None);
    }

    #[test]
    fn session_ended_update_should_close_the_ui() {
        let mut state = GuiState::default();
        state.apply(UiUpdate::SessionEnded);
        assert!(state.session_closed);
        assert_eq!(state.logs.len(), 1);
    }

    #[test]
    fn closed_session_should_be_logged_once() {
        let mut state = GuiState::default();
        state.mark_closed();
        state.mark_closed();
        assert!(state.session_closed);
        assert_eq!(state.logs.len(), 1);
    }
}
