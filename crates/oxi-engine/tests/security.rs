//! Aceptación de la fase 7: PIN cifrado en el TRQ (verificable con la misma
//! clave, lo que haría el HSM del host) y cambio de clave EKC (Tabla 10-12).

mod common;

use oxi_core::{parse_host, ParseOptions, RawFields, TerminalMessage};
use oxi_engine::{
    decode_decimal_key, decrypt_pin_block, encrypt_block, iso0_pin_block, kvv, Event, KeyMode,
    SecurityConfig, Terminal,
};

/// Clave de prueba sintética (NO usar fuera de tests).
const PIN_KEY_HEX: &str = "0F1E2D3C4B5A6978";
const MASTER_KEY_HEX: &str = "A1B2C3D4E5F60718";

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex del test"))
        .collect()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// El PIN viaja cifrado en el buffer A y el host lo descifra con la misma
/// clave: obtiene el ISO-0 exacto de (PIN, PAN).
#[test]
fn pin_should_travel_encrypted_and_decryptable_by_host() {
    let mut terminal = common::powered_and_loaded();
    terminal.configure_security(SecurityConfig {
        pin_key: Some(PIN_KEY_HEX.into()),
        ..SecurityConfig::default()
    });

    let (request, _mcn) = common::run_to_transaction_request(&mut terminal);
    let block = request.buffer_a.expect("buffer A presente");
    assert_eq!(block.len(), 16, "16 hex chars (§9-2 campo l)");

    let key = hex_to_bytes(PIN_KEY_HEX);
    let decrypted = decrypt_pin_block(&block, &key, KeyMode::Single).expect("descifra");
    let expected = iso0_pin_block("1234", &oxi_host_mock::test_card()).expect("iso0");
    assert_ne!(block, expected, "el bloque viaja cifrado");
    assert_eq!(decrypted, expected, "el host recupera el ISO-0");

    // KVV determinista para el host (§11-18).
    let terminal_kvv = terminal
        .pin_kvv()
        .expect("hay clave")
        .expect("KVV calculable");
    assert_eq!(terminal_kvv, kvv(&key, KeyMode::Single).expect("kvv"));
}

/// Extended Encryption Key Change modificador '2': la nueva clave de
/// comunicaciones viaja cifrada bajo la master y el terminal la instala.
#[test]
fn extended_key_change_should_install_new_comms_key() {
    // Clave nueva sintética (8 bytes) y su cifrado bajo la master.
    let new_key = hex_to_bytes("1020304050607080");
    let master = hex_to_bytes(MASTER_KEY_HEX);
    let block: [u8; 8] = new_key.clone().try_into().expect("8 bytes");
    let encrypted = encrypt_block(&master, block, KeyMode::Single).expect("cifra");
    // Datos decimales: 3 chars por byte (§10-45).
    let decimal: String = encrypted.iter().map(|byte| format!("{byte:03}")).collect();
    // Tabla 10-12: subclase '4' + modificador '2'; tamaño hex 018 = 8 bytes.
    let line = format!("3\x1c\x1c\x1c42\x1c018\x1c{decimal}");
    let raw = RawFields::parse(line.as_bytes());
    let message = parse_host(&raw, &ParseOptions::default()).expect("EKC sintético válido");

    let mut terminal = Terminal::new();
    let _ = terminal.handle(Event::PoweredOn);
    terminal.configure_security(SecurityConfig {
        master_key: Some(MASTER_KEY_HEX.into()),
        ..SecurityConfig::default()
    });
    // En Out-of-Service (tras PoweredOn) el EKC se acepta ('A').
    let actions = terminal.handle(Event::HostMessage(message));

    assert!(
        actions.iter().any(|action| matches!(
            action,
            oxi_engine::Action::Send(TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::Ready,
                ..
            })
        )),
        "responde Ready al cargar la clave: {actions:?}"
    );
    assert_eq!(
        terminal.security().pin_key.as_deref(),
        Some(bytes_to_hex(&new_key).as_str()),
        "la clave nueva quedó instalada"
    );
    // La clave decimal del mensaje equivalía al bloque cifrado.
    assert_eq!(
        decode_decimal_key(&decimal).expect("decimal válido"),
        encrypted
    );
}

/// En In-Service con transacción en curso, el EKC se rechaza (Tabla 10-38
/// 'A3': aceptar solo sin transacción).
#[test]
fn extended_key_change_during_transaction_should_be_rejected() {
    let mut terminal = common::powered_and_loaded();
    terminal.configure_security(SecurityConfig {
        master_key: Some(MASTER_KEY_HEX.into()),
        ..SecurityConfig::default()
    });
    let (_request, _mcn) = common::run_to_transaction_request(&mut terminal);
    // La transacción está esperando el reply → cualquier EKC debe rechazarse.
    let line = b"3\x1c\x1c\x1c42\x1c018\x1c000000000000000000000000";
    let raw = RawFields::parse(line);
    let message = parse_host(&raw, &ParseOptions::default()).expect("EKC sintético válido");
    let actions = terminal.handle(Event::HostMessage(message));
    assert!(actions.iter().any(|action| matches!(
        action,
        oxi_engine::Action::Send(TerminalMessage::SolicitedStatus {
            descriptor: oxi_core::StatusDescriptor::CommandReject,
            ..
        })
    )));
}
