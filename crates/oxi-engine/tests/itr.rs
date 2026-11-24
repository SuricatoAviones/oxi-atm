//! T8.1: Interactive Transaction Response (clase 3/2, manual §10-79).
//!
//! Flujo: TRQ normal → ITR con pantalla dinámica y teclas activas → el
//! cardholder responde → TRQ abreviado de Tabla 9-2 (solo buffer B).

mod common;

use std::time::Duration;

use oxi_core::{
    parse_host, HostMessage, ParseOptions, RawFields, StatusDescriptor, TerminalMessage,
};
use oxi_engine::{Action, Event, Key};

/// Construye una ITR sintética (sin header de transporte).
fn itr(display: char, active: &str, timer: &str, screen: &[u8]) -> HostMessage {
    let mut body = b"3\x1c\x1c\x1c2".to_vec();
    body.push(display as u8);
    body.extend_from_slice(active.as_bytes());
    body.extend_from_slice(b"\x1c");
    body.extend_from_slice(timer.as_bytes());
    if !screen.is_empty() {
        body.push(0x1c);
        body.extend_from_slice(screen);
    }
    parse_host(&RawFields::parse(&body), &ParseOptions::default()).expect("ITR sintética válida")
}

/// Teclas activas: numérico + FDK A + FDK B + Cancel.
const KEYS_NUMERIC_A_B_CANCEL: &str = "1110010000";

fn assert_specific_or_generic_reject(actions: &[Action]) {
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Send(TerminalMessage::SolicitedStatus {
                descriptor: StatusDescriptor::SpecificCommandReject
                    | StatusDescriptor::CommandReject,
                ..
            })
        )),
        "se esperaba un reject: {actions:?}"
    );
}

#[test]
fn itr_should_display_dynamic_screen_and_wait_for_keys() {
    let mut terminal = common::powered_and_loaded();
    let (_request, _mcn) = common::run_to_transaction_request(&mut terminal);

    let actions = terminal.handle(Event::HostMessage(itr(
        '1',
        KEYS_NUMERIC_A_B_CANCEL,
        "001",
        b"\x0cSELECCIONE CUENTA",
    )));

    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::DisplayData(data) if data == b"\x0cSELECCIONE CUENTA"
        )),
        "la pantalla dinámica debe mostrarse: {actions:?}"
    );
    assert!(actions
        .iter()
        .any(|action| matches!(action, Action::CancelTimer(3))));
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::StartTimer {
            timer: 98,
            after: Some(duration)
        } if *duration == Duration::from_millis(800)
    )));

    let echo = terminal.input_echo().expect("eco ITR activo");
    assert!(echo.text.is_empty());
    assert!(!echo.masked);
}

#[test]
fn itr_should_return_padded_buffer_b_on_enter() {
    let mut terminal = common::powered_and_loaded();
    let (request, mcn) = common::run_to_transaction_request(&mut terminal);
    let _ = terminal.handle(Event::HostMessage(itr(
        '1',
        KEYS_NUMERIC_A_B_CANCEL,
        "000",
        b"\x0cSELECCIONE CUENTA",
    )));

    for digit in ['1', '2'] {
        let _ = terminal.handle(Event::KeyPressed(Key::Digit(digit)));
    }
    let echo = terminal.input_echo().expect("eco ITR");
    assert_eq!(echo.text, "12");
    assert!(!echo.masked);

    let actions = terminal.handle(Event::KeyPressed(Key::Enter));
    let response = common::extract_trq(&actions);
    assert_eq!(response.buffer_b.as_deref(), Some("012"));
    assert_eq!(response.mcn, request.mcn, "la ITR conserva el MCN del TRQ");
    assert_eq!(response.mcn.as_char(), mcn);
    assert!(response.track2.is_none());
    assert!(response.amount.is_none());
    assert!(response.buffer_a.is_none());
    assert!(terminal.awaiting_reply());
    assert!(actions.iter().any(|action| matches!(
        action,
        Action::StartTimer {
            timer: 3,
            after: Some(_)
        }
    )));
}

#[test]
fn itr_fdk_selection_should_send_letter_in_buffer_b() {
    let mut terminal = common::powered_and_loaded();
    let _ = common::run_to_transaction_request(&mut terminal);
    // numérico + FDK B.
    let _ = terminal.handle(Event::HostMessage(itr(
        '1',
        "1010000000",
        "000",
        b"\x0cCUENTA?",
    )));

    let actions = terminal.handle(Event::KeyPressed(Key::Fdk(oxi_core::Fdk::B)));
    let response = common::extract_trq(&actions);
    assert_eq!(
        response.buffer_b.as_deref(),
        Some("00B"),
        "convención Q14: FDK como letra, relleno de ceros"
    );
}

#[test]
fn itr_cancel_should_append_e_and_display_star() {
    let mut terminal = common::powered_and_loaded();
    let _ = common::run_to_transaction_request(&mut terminal);
    // numérico + Cancel, display '*' (flag 2).
    let _ = terminal.handle(Event::HostMessage(itr(
        '2',
        "1000010000",
        "000",
        b"\x0cCANCELAR?",
    )));

    for digit in ['1', '2'] {
        let _ = terminal.handle(Event::KeyPressed(Key::Digit(digit)));
    }
    let echo = terminal.input_echo().expect("eco enmascarado");
    assert_eq!(echo.text, "**");
    assert!(echo.masked);

    let actions = terminal.handle(Event::KeyPressed(Key::Cancel));
    let response = common::extract_trq(&actions);
    assert_eq!(response.buffer_b.as_deref(), Some("12E"));
}

#[test]
fn itr_screen_timeout_should_append_t_and_resend_trq() {
    let mut terminal = common::powered_and_loaded();
    let _ = common::run_to_transaction_request(&mut terminal);
    let _ = terminal.handle(Event::HostMessage(itr(
        '1',
        "1000000000",
        "001",
        b"\x0cESPERANDO",
    )));

    let actions = terminal.handle(Event::TimerFired(98));
    let response = common::extract_trq(&actions);
    assert_eq!(response.buffer_b.as_deref(), Some("00T"));
    assert!(terminal.awaiting_reply());
}

#[test]
fn itr_outside_transaction_request_should_be_rejected() {
    let mut terminal = common::powered_and_loaded();
    let actions = terminal.handle(Event::HostMessage(itr(
        '1',
        "1000000000",
        "000",
        b"\x0cNO CORRESPONDE",
    )));
    assert_specific_or_generic_reject(&actions);
}

#[test]
fn itr_display_flag_zero_should_not_echo() {
    let mut terminal = common::powered_and_loaded();
    let _ = common::run_to_transaction_request(&mut terminal);
    let _ = terminal.handle(Event::HostMessage(itr(
        '0',
        "1000000000",
        "000",
        b"\x0cSIN ECO",
    )));
    let _ = terminal.handle(Event::KeyPressed(Key::Digit('7')));
    assert_eq!(terminal.input_echo(), None);
    assert_eq!(terminal.buffers().b, "7");
}
