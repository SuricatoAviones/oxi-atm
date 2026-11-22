//! `oxi-host-mock`: host falso NDC para probar oxi.
//!
//! Binario + librería: baja customización sintética canónica, responde
//! Transaction Requests (aprobar/rechazar) y registra los estados del ATM.
//! Habla `oxi-core` (sin `oxi-engine`): es un peer independiente.

mod fixtures;
mod framing;
mod host;

pub use fixtures::{customisation, test_card, transaction_reply};
pub use framing::{read_frame, write_frame};
pub use host::{HostConfig, HostError, HostReport, ScriptedHost};

/// Nombre del crate, para verificación de armado del workspace.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del crate debe ser `oxi-host-mock` (test humo de scaffolding).
    #[test]
    fn crate_name_should_be_oxi_host_mock() {
        assert_eq!(CRATE_NAME, "oxi-host-mock");
    }
}
