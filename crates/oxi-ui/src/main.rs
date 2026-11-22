//! oxi: simulador de cajeros NDC — CLI v1.
//!
//! Uso:
//! ```text
//! cargo run -p oxi-ui -- --config perfil.toml
//! ```
//! Comandos: `card <nombre>`, `pin <digitos>`, `amount <digitos>`,
//! `fdk <A-I>`, `enter`, `clear`, `cancel`, `cash`, `help`, `quit`.

use tokio::io::{AsyncBufReadExt, BufReader};

use oxi_core::Fdk;
use oxi_engine::{Event, Key};
use oxi_ui::runtime::{card_event, UiUpdate};
use oxi_ui::{spawn, SimConfig};

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

const HELP: &str = "\
comandos:
  card <nombre>     insertar una tarjeta del perfil
  pin <digitos>     teclear PIN y confirmar
  amount <digitos>  teclear importe y confirmar
  fdk <A-I>         pulsar una tecla funcional
  enter|clear|cancel
  cash              retirar el efectivo dispensado
  help | quit";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("oxi_net=info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let config = SimConfig::load(argument(&args, "--config").as_deref())?;
    println!("oxi: conectando a {} ...", config.host_addr());
    println!("{HELP}");

    let mut handle = spawn(&config);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    loop {
        tokio::select! {
            Some(update) = handle.updates.recv() => print_update(update),
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line == "quit" {
                            break;
                        }
                        if line == "help" {
                            println!("{HELP}");
                            continue;
                        }
                        match parse_command(line, &config) {
                            Some(events) => {
                                for event in events {
                                    if !handle.send(event).await {
                                        println!("runtime cerrado");
                                        return Ok(());
                                    }
                                }
                            }
                            None => println!("comando no reconocido (usá `help`)"),
                        }
                    }
                    _ => break,
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    println!("\ncerrando oxi...");
    handle.shutdown();
    Ok(())
}

fn print_update(update: UiUpdate) {
    match update {
        UiUpdate::Log(line) => println!("[log] {line}"),
        UiUpdate::Mode(mode) => println!("[modo] {mode:?}"),
        UiUpdate::Connection(state) => println!("[conexión] {state:?}"),
        UiUpdate::Screen { id, text } => {
            println!("--- pantalla {id:?} ---");
            if !text.is_empty() {
                println!("{text}");
            }
        }
    }
}

/// Traduce una línea de la CLI a uno o más eventos del motor.
fn parse_command(line: &str, config: &SimConfig) -> Option<Vec<Event>> {
    let mut parts = line.split_whitespace();
    let command = parts.next()?;
    match command {
        "card" => {
            let name = parts.next()?;
            let profile = config.cards.iter().find(|card| card.name == name)?;
            Some(vec![card_event(profile)])
        }
        "pin" | "amount" => {
            let digits = parts.next()?;
            if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let mut events: Vec<Event> = digits
                .chars()
                .map(|digit| Event::KeyPressed(Key::Digit(digit)))
                .collect();
            events.push(Event::KeyPressed(Key::Enter));
            Some(events)
        }
        "fdk" => {
            let fdk = Fdk::from_char(parts.next()?.chars().next()?).ok()?;
            Some(vec![Event::KeyPressed(Key::Fdk(fdk))])
        }
        "enter" => Some(vec![Event::KeyPressed(Key::Enter)]),
        "clear" => Some(vec![Event::KeyPressed(Key::Clear)]),
        "cancel" => Some(vec![Event::KeyPressed(Key::Cancel)]),
        "cash" => Some(vec![Event::CashTaken]),
        _ => None,
    }
}
