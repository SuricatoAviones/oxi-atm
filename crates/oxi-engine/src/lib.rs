//! `oxi-engine`: terminal NDC como máquina de estados pura.
//!
//! `Terminal::handle(Event) -> Vec<Action>`: entra un evento, salen acciones.
//! Sin tokio, sin red, sin reloj (ver `docs/DESIGN.md` §3). Depende solo de
//! `oxi-core`.

mod crypto;
mod event;
mod hardware;
mod host_ops;
mod state_exec;
mod terminal;

pub use crypto::{iso0_pin_block, CryptoError};
pub use event::{Action, BufferId, Card, Event, Key, Pending};
pub use hardware::{
    Cassette, DeviceCondition, DispenseOutcome, Dispenser, Fault, Hardware, PaperState, Printer,
    Reader, ReaderState, Sensors, CASSETTE_COUNT,
};
pub use terminal::{Buffers, Terminal};

/// Nombre del crate, para verificación de armado del workspace.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del crate debe ser `oxi-engine` (test humo de scaffolding).
    #[test]
    fn crate_name_should_be_oxi_engine() {
        assert_eq!(CRATE_NAME, "oxi-engine");
    }
}
