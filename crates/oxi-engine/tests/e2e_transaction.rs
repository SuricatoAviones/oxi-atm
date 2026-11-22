//! Aceptación de la fase 3 (ROADMAP): una transacción sintética completa
//! A→B→F→I→J solo con eventos, más el caso de MCN que no coincide.

mod common;

use common::{powered_and_loaded, run_to_transaction_request, send_reply};
use oxi_core::{StatusDescriptor, TerminalMessage};
use oxi_engine::{Action, Event};

#[test]
fn full_transaction_should_complete_from_events_only() {
    let mut terminal = powered_and_loaded();
    let (request, mcn) = run_to_transaction_request(&mut terminal);

    // Contenido del TRQ según la Tabla 9-1 y las opciones activas.
    assert_eq!(request.luno.as_str(), "001");
    assert_eq!(
        request.track2.as_deref(),
        Some(oxi_host_mock::test_card().as_str())
    );
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

    // Transaction Reply: aprobado, dispensar 1 billete del casete 1, recibo,
    // devolver tarjeta, next 005 (J).
    let actions = send_reply(&mut terminal, mcn, "005", "001000000000");
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

    let actions = send_reply(&mut terminal, wrong, "005", "001000000000");
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
