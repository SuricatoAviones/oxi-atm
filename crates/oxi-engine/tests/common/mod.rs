//! Fixture compartida por los tests de integración del engine.
//!
//! La customización viene del builder canónico de `oxi-host-mock` (T5.1):
//! este módulo solo la serializa y la pasa por el parser real del engine,
//! así los tests siguen ejercitando `parse_host`.
#![allow(dead_code)]

use oxi_core::{parse_host, ParseOptions, RawFields, TerminalMessage};
use oxi_engine::{Action, Card, Event, Key, Terminal};

pub fn load(terminal: &mut Terminal, line: &[u8]) {
    let raw = RawFields::parse(line);
    let message = parse_host(&raw, &ParseOptions::default()).expect("mensaje sintético válido");
    let _ = terminal.handle(Event::HostMessage(message));
}

/// Carga la customización canónica (pantallas/estados/FIT/ECPL/Config ID).
pub fn load_customisation(terminal: &mut Terminal) {
    for message in oxi_host_mock::customisation() {
        let bytes = message.to_bytes().expect("fixture serializable");
        load(terminal, &bytes);
    }
}

pub fn powered_and_loaded() -> Terminal {
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

pub fn extract_trq(actions: &[Action]) -> oxi_core::TransactionRequest {
    actions
        .iter()
        .find_map(|action| match action {
            Action::Send(TerminalMessage::TransactionRequest(request)) => Some((**request).clone()),
            _ => None,
        })
        .expect("el estado I debe enviar un TRQ")
}

/// Flujo completo hasta el TRQ (tarjeta + PIN + importe).
pub fn run_to_transaction_request(terminal: &mut Terminal) -> (oxi_core::TransactionRequest, char) {
    let _ = terminal.handle(Event::CardInserted(Card {
        track2: Some(oxi_host_mock::test_card()),
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

/// Construye un Transaction Reply sintético con el MCN dado.
pub fn reply_line(next: &str, notes: &str, mcn: char) -> String {
    format!("4\x1c\x1c\x1c{next}\x1c{notes}\x1c00012052\x1c{mcn}01RECIBO")
}

pub fn send_reply(terminal: &mut Terminal, mcn: char, next: &str, notes: &str) -> Vec<Action> {
    let line = reply_line(next, notes, mcn);
    let reply = parse_host(&RawFields::parse(line.as_bytes()), &ParseOptions::default())
        .expect("reply sintético válido");
    terminal.handle(Event::HostMessage(reply))
}

pub fn find_unsolicited(actions: &[Action]) -> Option<Vec<String>> {
    actions.iter().find_map(|action| match action {
        Action::Send(TerminalMessage::UnsolicitedStatus { data, .. }) => Some(data.clone()),
        _ => None,
    })
}

pub fn find_device_fault(actions: &[Action]) -> Option<Vec<String>> {
    actions.iter().find_map(|action| match action {
        Action::Send(TerminalMessage::SolicitedStatus {
            descriptor: oxi_core::StatusDescriptor::DeviceFault,
            data,
            ..
        }) => Some(data.clone()),
        _ => None,
    })
}
