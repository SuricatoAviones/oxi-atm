//! Host falso NDC: baja customización sintética, pone el ATM en servicio y
//! responde transacciones. Útil para la demo manual (fase 6) y los tests.
//!
//! Uso:
//! ```text
//! cargo run -p oxi-host-mock -- --listen 127.0.0.1:9000 --approve
//! ```

use std::time::Duration;

use oxi_host_mock::{HostConfig, ScriptedHost};

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let listen = argument(&args, "--listen").unwrap_or_else(|| "127.0.0.1:9000".into());
    let approve = !args.iter().any(|arg| arg == "--reject");
    // Silencio máximo entre mensajes del ATM. En la demo manual el operador
    // necesita tiempo para insertar la tarjeta; los tests pasan su propio
    // valor corto vía la librería.
    let idle_secs = argument(&args, "--idle")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(1800);

    let host = ScriptedHost::bind(&listen).await?;
    let addr = host.local_addr()?;
    println!(
        "oxi-host-mock escuchando en {addr} ({}, silencio máximo {idle_secs}s)",
        if approve { "aprobar" } else { "rechazar" }
    );
    println!("esperando la conexión del ATM (oxi)...");

    loop {
        let config = HostConfig {
            approve,
            idle_timeout: Duration::from_secs(idle_secs),
        };
        match host.run_once(&config).await {
            Ok(report) => println!(
                "sesión: {} frames ↓ / {} frames ↑, {} transacción(es), statuses={:?}",
                report.frames_out, report.frames_in, report.transactions, report.statuses
            ),
            Err(err) => eprintln!("sesión terminada con error: {err}"),
        }
        println!("esperando otra conexión...");
    }
}
