//! oxi-gui: interfaz gráfica del simulador NDC.
//!
//! Uso:
//! ```text
//! cargo run -p oxi-ui --features gui --bin oxi-gui -- --config perfil.toml
//! ```
//! Requiere una ventana (Windows nativo o WSLg en WSL). La CLI (`oxi-ui`)
//! sigue disponible y comparte exactamente el mismo runtime.

use oxi_ui::{gui, SimConfig};

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("oxi_net=info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let config = SimConfig::load(argument(&args, "--config").as_deref())?;
    gui::run(config)
}
