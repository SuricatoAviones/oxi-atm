//! Herramienta de verificación local: parsea un archivo de mensajes NDC
//! (uno por línea, con escapes `#NN` para caracteres de control) e informa la
//! clasificación de cada uno. Uso:
//!
//! ```text
//! cargo run -p oxi-core --example parse_file -- ruta/al/archivo.txt
//! ```
//!
//! No contiene datos: el archivo lo aporta quien lo invoca (p.ej. el download
//! real, que vive fuera del repo por confidencialidad).

use std::env;
use std::fmt::Write as _;

use oxi_core::{parse_host, CustomisationData, HostMessage, ParseOptions, RawFields};

/// Decodifica la notación `#NN` (decimal) a bytes crudos.
fn decode_escapes(line: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 2 < bytes.len() + 1 && i + 2 < bytes.len() + 1 {
            let hex = &line[i + 1..(i + 3).min(line.len())];
            if let Ok(value) = hex.parse::<u8>() {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn describe(msg: &HostMessage) -> String {
    match msg {
        HostMessage::TerminalCommand(cmd) => format!("TerminalCommand({cmd:?})"),
        HostMessage::Customisation(data) => {
            let mut desc = String::new();
            let name = match data {
                CustomisationData::Screens(v) => {
                    let _ = write!(desc, "{} pantalla(s)", v.len());
                    "Screens"
                }
                CustomisationData::StateTables(v) => {
                    let _ = write!(
                        desc,
                        "{} estado(s): {}",
                        v.len(),
                        v.iter()
                            .map(|t| format!("{}{}", t.number, t.state_type))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    "StateTables"
                }
                CustomisationData::ConfigParams(p) => {
                    let _ = write!(
                        desc,
                        "options={:03} luno={:?} timers={}",
                        p.options_mask,
                        p.luno,
                        p.timers.len()
                    );
                    "ConfigParams"
                }
                CustomisationData::Fits(v) => {
                    let _ = write!(
                        desc,
                        "{} FIT(s): {}",
                        v.len(),
                        v.iter()
                            .map(|f| format!("#{} ({} entradas)", f.number, f.entries.len()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    "Fits"
                }
                CustomisationData::ConfigId(id) => {
                    let _ = write!(desc, "{id:04}");
                    "ConfigId"
                }
                CustomisationData::EnhancedConfig(e) => {
                    let _ = write!(
                        desc,
                        "luno={:?} opciones={} -> {:?}",
                        e.luno,
                        e.options.len(),
                        e.options
                            .iter()
                            .map(|o| format!("{:02}/{:03}", o.number, o.code))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                    "EnhancedConfig"
                }
                CustomisationData::MacFieldSelection(f) => {
                    let _ = write!(desc, "{} campo(s) crudo(s)", f.len());
                    "MacFieldSelection(raw)"
                }
                CustomisationData::DateTime(dt) => {
                    let _ = write!(
                        desc,
                        "{:04}-{:02}-{:02} {:02}:{:02}{}",
                        dt.year,
                        dt.month,
                        dt.day,
                        dt.hour,
                        dt.minute,
                        if dt.quirk_14 { " [quirk 14]" } else { "" }
                    );
                    "DateTime"
                }
                CustomisationData::CassetteMapping(f) => {
                    let _ = write!(desc, "{} campo(s) crudo(s)", f.len());
                    "CassetteMapping(raw)"
                }
                CustomisationData::OverrideReservedScreens(f) => {
                    let _ = write!(desc, "{} campo(s) crudo(s)", f.len());
                    "OverrideReservedScreens(raw)"
                }
            };
            format!("Customisation::{name} [{desc}]")
        }
        HostMessage::TransactionReply(tr) => {
            format!(
                "TransactionReply(next={} fn={:?} notas={:?} pantalla={:?} mcn={:?})",
                tr.next_state, tr.function_id, tr.notes, tr.screen, tr.mcn
            )
        }
        HostMessage::InteractiveTransactionResponse(itr) => {
            format!(
                "ITR(flag='{}' teclas='{}' timer={:?} pantalla={} bytes)",
                itr.display_flag,
                itr.active_keys,
                itr.screen_timer,
                itr.screen_data.len()
            )
        }
        HostMessage::EncryptionKeyChange { .. } => "EncryptionKeyChange (crudo; fase 7)".to_owned(),
        HostMessage::ExtendedEncryptionKeyChange { .. } => {
            "ExtendedEncryptionKeyChange (crudo; fase 7)".to_owned()
        }
        HostMessage::Unknown { .. } => "Unknown (preservado; el motor decide reject)".to_owned(),
    }
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("uso: parse_file <ruta a archivo de mensajes, uno por línea>");
        std::process::exit(2);
    });
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("no se pudo leer '{path}': {err}"));

    let options = ParseOptions::default();
    let mut ok = 0usize;
    let mut errors = 0usize;
    for (index, line) in content.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let decoded = decode_escapes(line.trim_end());
        let raw = RawFields::parse(&decoded);
        match parse_host(&raw, &options) {
            Ok(msg) => {
                ok += 1;
                println!("L{:02} OK   {}", index + 1, describe(&msg));
            }
            Err(err) => {
                errors += 1;
                println!("L{:02} FAIL {}", index + 1, err);
            }
        }
    }
    println!("--- {} OK / {} FAIL / modo estricto ---", ok, errors);
}
