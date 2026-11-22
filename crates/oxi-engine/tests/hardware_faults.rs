//! Tests del hardware virtual y el catálogo de fallas (fase 4; Q9).

mod common;

use common::{
    find_device_fault, find_unsolicited, powered_and_loaded, run_to_transaction_request, send_reply,
};
use oxi_engine::{Action, Event, Fault};

/// Falla de impresora de recibo: unsolicited 'G' con supplies 3111
/// (ejemplo de la Tabla 9-33) y la transacción continúa.
#[test]
fn receipt_paper_out_should_report_unsolicited_g_3111() {
    let mut terminal = powered_and_loaded();
    let actions = terminal.inject_fault(Fault::ReceiptPaperOut);
    let data = find_unsolicited(&actions).expect("unsolicited inmediato");
    assert_eq!(data[0], "G0", "DIG 'G' + status '0'");
    assert_eq!(data[1], "4", "severidad fatal");
    assert_eq!(data[3], "3111", "papel agotado, ribbon/head/knife OK");
}

/// Atasco del dispensador: el TR recibe device fault 'E' con T-code '2'
/// (sin notas) y severidad 4.
#[test]
fn cash_jam_should_report_device_fault_and_dispense_nothing() {
    let mut terminal = powered_and_loaded();
    let _ = terminal.inject_fault(Fault::CashJam);
    let (_request, mcn) = run_to_transaction_request(&mut terminal);
    let actions = send_reply(&mut terminal, mcn, "005", "001000000000");
    let fault = find_device_fault(&actions).expect("device fault solicitado");
    assert!(
        fault[0].starts_with("E2"),
        "cash handler + T-code 2 (no dispensó): {}",
        fault[0]
    );
    assert_eq!(fault[1], "4");
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Dispense(_))),
        "no debe dispensar nada"
    );
}

/// Dispensado parcial: entrega la mitad (T-code '1'), device fault y el
/// flujo de retiro sigue su curso.
#[test]
fn partial_dispense_should_deliver_half_and_fault() {
    let mut terminal = powered_and_loaded();
    let _ = terminal.inject_fault(Fault::CashPartial);
    let (_request, mcn) = run_to_transaction_request(&mut terminal);
    // Pedir 2 notas: el parcial entrega la mitad (1) → Short.
    let actions = send_reply(&mut terminal, mcn, "005", "002000000000");
    let fault = find_device_fault(&actions).expect("device fault por short");
    assert!(fault[0].starts_with("E1"), "short dispense: {}", fault[0]);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Dispense(notes) if notes.total() == 1
        )),
        "entrega la mitad disponible: {actions:?}"
    );
}

/// Send Supply Counters: responde Terminal State '2' con la Tabla 9-8
/// (TSN 4 + acumuladas 7 + 4×20 + capturadas/sobres/cámara/serial).
#[test]
fn supply_counters_should_follow_table_9_8() {
    let mut terminal = powered_and_loaded();
    let (request, mcn) = run_to_transaction_request(&mut terminal);
    let _ = send_reply(&mut terminal, mcn, "005", "001000000000");
    let _ = terminal.handle(Event::CashTaken);

    // Tabla 10-40: los contadores ('D'/'P'/'R') se rechazan In-Service;
    // hay que pasar a Out-of-Service primero.
    let _ = terminal.handle(Event::HostMessage(
        oxi_core::parse_host(
            &oxi_core::RawFields::parse(b"1\x1c\x1c\x1c2"),
            &oxi_core::ParseOptions::default(),
        )
        .expect("go out-of-service válido"),
    ));
    let counters = terminal.handle(Event::HostMessage(
        oxi_core::parse_host(
            &oxi_core::RawFields::parse(b"1\x1c\x1c\x1c4"),
            &oxi_core::ParseOptions::default(),
        )
        .expect("comando 4 válido"),
    ));
    let data = counters
        .iter()
        .find_map(|action| match action {
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::TerminalState,
                data,
                ..
            }) => Some(data[0].clone()),
            _ => None,
        })
        .expect("debe responder Terminal State");
    let payload = &data[1..];
    assert!(data.starts_with('2'), "g1 = '2'");
    assert_eq!(payload.len(), 4 + 7 + 20 * 4 + 5 + 5 + 5 + 5);
    assert_eq!(&payload[..4], "0001", "TSN del reply");
    assert_eq!(&payload[4..11], "0000001", "un intento acumulado");
    // Notas en casete 1: 100 - 1 dispensada = 99.
    assert_eq!(&payload[11..16], "00099");
    let status_data = request.status_data.as_deref().expect("opción 15 activa");
    assert_eq!(&status_data[..4], "0000");
}

/// Timer 09: tarjeta presentada y no retirada → captura + unsolicited 'D1'.
#[test]
fn card_not_taken_should_be_captured_on_timer_9() {
    let mut terminal = powered_and_loaded();
    let (_request, mcn) = run_to_transaction_request(&mut terminal);
    let _ = send_reply(&mut terminal, mcn, "005", "001000000000");
    let _ = terminal.handle(Event::CashTaken);
    // El J dejó la tarjeta presentada con timer 09.
    let actions = terminal.handle(Event::TimerFired(9));
    let data = find_unsolicited(&actions).expect("unsolicited de captura");
    assert_eq!(data[0], "D1", "card reader + tarjeta capturada");
}

/// Timer 07: efectivo no retirado → retract + unsolicited 'E5'.
#[test]
fn cash_not_taken_should_be_retracted_on_timer_7() {
    let mut terminal = powered_and_loaded();
    let (_request, mcn) = run_to_transaction_request(&mut terminal);
    let _ = send_reply(&mut terminal, mcn, "005", "001000000000");
    let actions = terminal.handle(Event::TimerFired(7));
    let data = find_unsolicited(&actions).expect("unsolicited de retract");
    assert!(
        data[0].starts_with("E5"),
        "T-code 5: notas retraídas en {}",
        data[0]
    );
}

/// Modo supervisor: entrada reporta 'P21' (Tabla 9-41).
#[test]
fn supervisor_mode_entry_should_report_p21() {
    let mut terminal = powered_and_loaded();
    let actions = terminal.set_supervisor_mode(true);
    let data = find_unsolicited(&actions).expect("unsolicited de modo");
    assert_eq!(data[0], "P21");
}
