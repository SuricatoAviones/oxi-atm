//! Aceptación de la fase 3 (ROADMAP): una transacción sintética completa
//! A→B→F→I→J solo con eventos, más el caso de MCN que no coincide.
//!
//! Usa exclusivamente la API pública del motor y fixtures sintéticas (los
//! mensajes se construyen byte a byte acá; nada de datos reales).

use oxi_core::{parse_host, ParseOptions, RawFields, StatusDescriptor, TerminalMessage};
use oxi_engine::{Action, Card, Event, Key, Terminal};

fn load(terminal: &mut Terminal, line: &[u8]) {
    let raw = RawFields::parse(line);
    let message = parse_host(&raw, &ParseOptions::default()).expect("mensaje sintético válido");
    let _ = terminal.handle(Event::HostMessage(message));
}

/// Customización sintética: pantallas, estados A/K/B/F/I/J, FIT comodín y
/// ECPL con LUNO 001 + opciones 12 (specific reject) y 15 (status data).
fn load_customisation(terminal: &mut Terminal) {
    // Pantallas: 010 bienvenida, 020 importe, 028 PIN, 052 recibo.
    let screens: &[u8] =
        b"3\x1c\x1c\x1c11\x1c010\x0cBIENVENIDO\x1c020\x0cIMPORTE\x1c028\x0cPIN\x1c052\x0cRECIBO";
    load(terminal, screens);

    // State tables (layouts del manual §2-4/§2-8/§2-17/§2-25/§2-30):
    let states: &[u8] = b"3\x1c\x1c\x1c12\x1c\
000A010001001002002002000001\x1c\
001K002002002002002002002002\x1c\
002B028200201010213032003003\x1c\
003F020200200004255255255020\x1c\
004I023203001000001001001000\x1c\
005J052000052052052052052\x1c\
200J052255052052052052052\x1c\
201J052255052052052052052\x1c\
203J052255052052052052052";
    load(terminal, states);

    // FIT comodín: número 001, PIDDX 000, PFIID FF×5, resto ceros
    // (40 entradas: el formato del driver real, SPEC Q11).
    let fit_data = format!("001000{}{}", "255".repeat(5), "000".repeat(34));
    let fit_line = format!("3\x1c\x1c\x1c15\x1c{fit_data}");
    load(terminal, fit_line.as_bytes());

    // ECPL: LUNO 001 + opción 12/001 + opción 15/001.
    load(terminal, b"3\x1c\x1c\x1c1A\x1c001\x1c1200115001");
}

fn powered_and_loaded() -> Terminal {
    let mut terminal = Terminal::new();
    let _ = terminal.handle(Event::PoweredOn);
    load_customisation(&mut terminal);
    let _ = terminal.handle(Event::HostMessage(
        parse_host(
            &RawFields::parse(b"1\x1c\x1c\x1c1"),
            &ParseOptions::default(),
        )
        .expect("go in-service válido"),
    ));
    terminal
}

/// Extrae el TRQ enviado de la lista de acciones.
fn extract_trq(actions: &[Action]) -> oxi_core::TransactionRequest {
    actions
        .iter()
        .find_map(|action| match action {
            Action::Send(TerminalMessage::TransactionRequest(request)) => Some((**request).clone()),
            _ => None,
        })
        .expect("el estado I debe enviar un TRQ")
}

/// Flujo completo hasta el TRQ (tarjeta + PIN + importe).
fn run_to_transaction_request(terminal: &mut Terminal) -> (oxi_core::TransactionRequest, char) {
    let _ = terminal.handle(Event::CardInserted(Card {
        track2: Some("5899410000000000=2512".into()),
        ..Card::default()
    }));
    assert_eq!(
        terminal.current_state().map(|state| state.value()),
        Some(2),
        "A → K → B"
    );

    for digit in ['1', '2', '3', '4'] {
        let _ = terminal.handle(Event::KeyPressed(Key::Digit(digit)));
    }
    let _ = terminal.handle(Event::KeyPressed(Key::Enter));
    assert!(
        terminal.buffers().pin_block.is_some(),
        "PIN block formado al confirmar"
    );
    assert_eq!(
        terminal.current_state().map(|state| state.value()),
        Some(3),
        "B → F"
    );

    for digit in ['1', '0', '0'] {
        let _ = terminal.handle(Event::KeyPressed(Key::Digit(digit)));
    }
    let actions = terminal.handle(Event::KeyPressed(Key::Enter));
    let request = extract_trq(&actions);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::StartTimer { timer: 3, .. })),
        "el TRQ arranca el timer 03"
    );
    let mcn = request.mcn.as_char();
    (request, mcn)
}

#[test]
fn full_transaction_should_complete_from_events_only() {
    let mut terminal = powered_and_loaded();
    let (request, mcn) = run_to_transaction_request(&mut terminal);

    // Contenido del TRQ según la Tabla 9-1 y las opciones activas.
    assert_eq!(request.luno.as_str(), "001");
    assert_eq!(request.track2.as_deref(), Some("5899410000000000=2512"));
    assert_eq!(request.amount.as_deref(), Some("00000100"));
    assert_eq!(request.op_code.as_deref(), Some("        "));
    assert_eq!(
        request.buffer_a.as_ref().map(String::len),
        Some(16),
        "buffer A = PIN block de 16 hex"
    );
    assert_eq!(
        request.status_data.as_ref().map(String::len),
        Some(25),
        "opción 15: serial(4)+status(1)+notas(20)"
    );

    // Transaction Reply sintético: aprobado, dispensar 1 billete, recibo,
    // devolver tarjeta, next 005 (J).
    let reply_line = format!("4\x1c\x1c\x1c005\x1c001000000000\x1c00012052\x1c{mcn}01RECIBO");
    let reply = parse_host(
        &RawFields::parse(reply_line.as_bytes()),
        &ParseOptions::default(),
    )
    .expect("reply sintético válido");
    let actions = terminal.handle(Event::HostMessage(reply));

    // Dispensado + impresión; Ready y transición diferidos al retiro.
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Dispense(notes) if notes.total() == 1
        )),
        "debe dispensar 1 billete: {actions:?}"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Print { printer: oxi_core::PrinterFlag::Journal, data } if data == "RECIBO"
        )),
        "debe imprimir el grupo de journal"
    );
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::StartTimer { timer: 7, .. })),
        "arranca el timer de retiro"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            Action::Send(TerminalMessage::SolicitedStatus { .. })
        )),
        "Ready se difiere hasta el retiro del efectivo"
    );

    // El cardholder retira: Ready + J (recibo, devolver tarjeta, volver a A).
    let actions = terminal.handle(Event::CashTaken);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Send(TerminalMessage::SolicitedStatus {
                descriptor: StatusDescriptor::Ready,
                ..
            })
        )),
        "Ready tras el retiro"
    );
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::EjectCard)),
        "J devuelve la tarjeta (bandera '0')"
    );
    assert_eq!(
        terminal.current_state().map(|state| state.value()),
        Some(0),
        "J 005 vuelve al estado A"
    );
}

/// MCN que no coincide: specific reject B09 (opción 12 activa) y la
/// transacción sigue esperando el reply correcto.
#[test]
fn mismatched_mcn_should_specific_reject_b09() {
    let mut terminal = powered_and_loaded();
    let (_request, mcn) = run_to_transaction_request(&mut terminal);
    let wrong = if mcn == '2' { '3' } else { '2' };

    let reply_line = format!("4\x1c\x1c\x1c005\x1c001000000000\x1c00012052\x1c{wrong}01RECIBO");
    let reply = parse_host(
        &RawFields::parse(reply_line.as_bytes()),
        &ParseOptions::default(),
    )
    .expect("reply sintético válido");
    let actions = terminal.handle(Event::HostMessage(reply));

    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Send(TerminalMessage::SolicitedStatus {
                descriptor: StatusDescriptor::SpecificCommandReject,
                data,
                ..
            }) if data == &vec!["B09".to_string()]
        )),
        "debe responder specific reject B09: {actions:?}"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Dispense(_))),
        "no debe dispensar con MCN inválido"
    );
}
