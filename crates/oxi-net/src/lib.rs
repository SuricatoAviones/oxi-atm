//! `oxi-net`: transporte TCP del simulador (ATM → host) con tokio.
//!
//! El ATM **inicia** la conexión hacia el host (rol cliente, SPEC §1). Este
//! crate trabaja con payloads enmarcados (bytes completos de mensaje); el
//! parseo NDC vive en `oxi-core` — una sola fuente de verdad del formato.

mod codec;
mod connect;
mod error;

pub use codec::{FrameCodec, LenPrefix2Be};
pub use connect::{connect_with_backoff, ConnectionState, ReconnectConfig};
pub use error::NetError;

/// Nombre del crate, para verificación de armado del workspace.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del crate debe ser `oxi-net` (test humo de scaffolding).
    #[test]
    fn crate_name_should_be_oxi_net() {
        assert_eq!(CRATE_NAME, "oxi-net");
    }
}
