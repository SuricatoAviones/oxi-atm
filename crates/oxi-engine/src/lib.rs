//! `oxi-engine`: terminal NDC como máquina de estados pura.
//!
//! `Terminal::handle(Event) -> Vec<Action>`: entra un evento, salen acciones.
//! Sin tokio, sin red, sin reloj (ver `docs/DESIGN.md` §3). Depende solo de
//! `oxi-core`.

// TEMPORAL (T3.1-T3.3): helpers del intérprete que T3.4-T3.7 van a usar y
// todavía no tienen llamador en el build no-test. Se ELIMINA al completar la
// fase: cuando se quite, el compilador marcará cualquier helper que haya
// quedado sin uso.
#![allow(dead_code)]

mod event;
mod host_ops;
mod state_exec;
mod terminal;

pub use event::{Action, BufferId, Card, Event, Key, Pending};
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
