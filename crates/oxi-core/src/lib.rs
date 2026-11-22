//! `oxi-core`: tipos y parser/serializador de mensajes NDC (APTRA Advance NDC).
//!
//! Este crate es la única fuente de verdad del formato del cable: mensajes como
//! enums, split por FS/GS y roundtrips verificados. No depende de tokio, I/O ni
//! interfaz (ver `docs/ARCHITECTURE.md` §2).

/// Nombre del crate, para verificación de armado del workspace.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del crate debe ser `oxi-core` (test humo de scaffolding).
    #[test]
    fn crate_name_should_be_oxi_core() {
        assert_eq!(CRATE_NAME, "oxi-core");
    }
}
