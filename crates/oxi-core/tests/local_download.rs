//! Verificación local del download real del dueño — OPT-IN por variable de
//! entorno, para no versionar ni depender de datos confidenciales en CI:
//!
//! ```text
//! OXI_LOCAL_DOWNLOAD="reference/download/downloadNDC.txt" cargo test -p oxi-core
//! ```
//!
//! Sin la variable, el test se salta. Con ella, cada línea del archivo debe
//! parsear conforme al manual, con las dos excepciones documentadas en SPEC
//! §14 (Q12: la plantilla trae Date/Time vacío; se espera FAIL estricto en
//! esa línea y solo en esa).

use oxi_core::{parse_host, HostMessage, ParseOptions, RawFields};

/// Decodifica la notación `#NN` del archivo de download del dueño.
fn decode_escapes(line: &str) -> Vec<u8> {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 3 <= bytes.len() {
            if let Ok(value) = line[i + 1..i + 3].parse::<u8>() {
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

#[test]
fn local_download_should_parse_per_manual() {
    let Ok(path) = std::env::var("OXI_LOCAL_DOWNLOAD") else {
        // Sin la variable: skip (la confidencialidad manda; CI sin datos reales).
        return;
    };
    // El harness corre con CWD en el crate: resolvemos contra la raíz del
    // workspace para que las rutas relativas del repo funcionen igual.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/oxi-core está a dos niveles de la raíz");
    let full = workspace_root.join(path.trim_start_matches(std::path::MAIN_SEPARATOR));
    let content = std::fs::read_to_string(&full)
        .unwrap_or_else(|err| panic!("no se pudo leer '{full:?}': {err}"));
    let options = ParseOptions::default();

    let mut ok = 0;
    let mut expected_fail = 0;
    let mut unexpected = Vec::new();
    for (index, line) in content.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let line_no = index + 1;
        let raw = RawFields::parse(&decode_escapes(line.trim_end()));
        match parse_host(&raw, &options) {
            Ok(msg) => {
                // La plantilla del download trae el Date/Time vacío (SPEC Q12):
                // esa línea es la única que debe fallar en modo estricto, y
                // este brazo no la alcanza.
                ok += 1;
                if matches!(msg, HostMessage::Customisation(_))
                    || matches!(msg, HostMessage::TerminalCommand(_))
                {
                    // clasificación válida
                }
            }
            Err(_) => {
                expected_fail += 1;
                // Registramos qué líneas fallaron para asertar después.
                unexpected.push(line_no);
            }
        }
    }

    // El download de nov-2026 trae 44 líneas: 43 conformes + 1 Date/Time vacío
    // (Q12, plantilla sin la fecha que el driver inserta en runtime).
    assert_eq!(ok, 43, "líneas conformes: {ok}");
    assert_eq!(
        unexpected,
        vec![2],
        "solo la línea 2 (Date/Time vacío, SPEC Q12) debe fallar; fallaron: {unexpected:?}"
    );
    let _ = expected_fail;
}
