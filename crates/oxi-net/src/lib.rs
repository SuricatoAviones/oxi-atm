//! `oxi-net`: transporte TCP del simulador (ATM → host) con tokio.
//!
//! Codec de encuadre configurable (default: prefijo de 2 bytes big-endian,
//! confirmado por el dueño, SPEC §1) y reconexión con backoff. No conoce
//! `oxi-engine`: habla el idioma de `oxi-core` (bytes de mensajes).

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
