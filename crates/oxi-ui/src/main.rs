//! `oxi-ui`: ensamblado del simulador e interfaz (CLI en v1).
//!
//! Glue de runtime: conecta `oxi-net` ↔ `oxi-engine` ↔ teclado/pantalla vía
//! canales de tokio (ver `docs/DESIGN.md` §6). La GUI llega después del
//! núcleo, con aprobación previa.

fn main() {
    println!("oxi: simulador NDC (fase 6 del ROADMAP)");
}

#[cfg(test)]
mod tests {
    /// El nombre del binario debe ser `oxi-ui` (test humo de scaffolding).
    #[test]
    fn binary_name_should_be_oxi_ui() {
        assert_eq!(env!("CARGO_PKG_NAME"), "oxi-ui");
    }
}
