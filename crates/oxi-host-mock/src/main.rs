//! `oxi-host-mock`: host falso NDC para probar oxi.
//!
//! Binario + librería: baja customización sintética, responde Transaction
//! Requests (modo automático/moderado) y simula fallas de dispositivos.
//! Los fixtures se construyen con código propio (regla de AGENTS.md).

fn main() {
    println!("oxi-host-mock: host falso NDC (fase 5 del ROADMAP)");
}

#[cfg(test)]
mod tests {
    /// El nombre del binario debe ser `oxi-host-mock` (test humo de scaffolding).
    #[test]
    fn binary_name_should_be_oxi_host_mock() {
        assert_eq!(env!("CARGO_PKG_NAME"), "oxi-host-mock");
    }
}
