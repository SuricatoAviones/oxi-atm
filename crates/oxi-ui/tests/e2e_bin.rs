//! Aceptación de la fase 6: runtime + red + mock, todo local y sintético.
//!
//! `oxi-ui` (runtime completo: net + engine + timers) contra
//! `oxi-host-mock` (host falso) por TCP real en localhost.

use std::time::Duration;

use oxi_core::OperationalMode;
use oxi_engine::{Event, Key};
use oxi_host_mock::{HostConfig, ScriptedHost};
use oxi_ui::runtime::{card_event, UiUpdate};
use oxi_ui::{spawn, SimConfig};
use tokio::sync::mpsc;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

async fn wait_for<F>(updates: &mut mpsc::Receiver<UiUpdate>, predicate: F) -> UiUpdate
where
    F: Fn(&UiUpdate) -> bool,
{
    timeout(WAIT, async {
        while let Some(update) = updates.recv().await {
            if predicate(&update) {
                return update;
            }
        }
        panic!("runtime cerrado");
    })
    .await
    .expect("timeout esperando actualización")
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_should_complete_a_transaction_against_the_mock() {
    // Host falso en un puerto efímero.
    let host = ScriptedHost::bind("127.0.0.1:0").await.expect("bind");
    let addr = host.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        host.run_once(&HostConfig {
            approve: true,
            idle_timeout: Duration::from_secs(5),
        })
        .await
    });

    let mut config = SimConfig::default();
    config.net.host = "127.0.0.1".into();
    config.net.port = addr.port();
    let mut handle = spawn(&config);

    // 1) La descarga completa deja al terminal en servicio.
    wait_for(&mut handle.updates, |update| {
        matches!(update, UiUpdate::Mode(OperationalMode::InService))
    })
    .await;

    // 2) Tarjeta → PIN → importe (Enter = FDK A).
    let card = config.cards[0].clone();
    assert!(handle.send(card_event(&card)).await);
    for digit in ['1', '2', '3', '4'] {
        assert!(handle.send(Event::KeyPressed(Key::Digit(digit))).await);
    }
    assert!(handle.send(Event::KeyPressed(Key::Enter)).await);
    for digit in ['1', '0', '0'] {
        assert!(handle.send(Event::KeyPressed(Key::Digit(digit))).await);
    }
    assert!(handle.send(Event::KeyPressed(Key::Enter)).await);

    // 3) El host aprueba: llegan dispensado e impresión de "APROBADO".
    let mut saw_dispense = false;
    let mut saw_approved_print = false;
    let mut saw_pin_echo = false;
    let mut saw_amount_echo = false;
    timeout(WAIT, async {
        while !(saw_dispense && saw_approved_print) {
            let Some(update) = handle.updates.recv().await else {
                panic!("runtime cerrado");
            };
            match &update {
                UiUpdate::Log(line) => {
                    saw_dispense |= line.contains("dispensando");
                    saw_approved_print |= line.contains("APROBADO");
                }
                // Eco local del PIN (§2-8) y del importe (§2-17).
                UiUpdate::InputEcho(Some(echo)) if echo.masked && echo.text == "XXXX" => {
                    saw_pin_echo = true;
                }
                UiUpdate::InputEcho(Some(echo)) if !echo.masked && echo.text == "100" => {
                    saw_amount_echo = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timeout esperando la respuesta del host");
    assert!(saw_pin_echo, "el runtime debe emitir el eco del PIN");
    assert!(saw_amount_echo, "el runtime debe emitir el eco del importe");

    // 4) Retiro del efectivo: Ready + cierre (tarjeta devuelta).
    assert!(handle.send(Event::CashTaken).await);
    wait_for(
        &mut handle.updates,
        |update| matches!(update, UiUpdate::Log(line) if line.contains("tarjeta devuelta")),
    )
    .await;

    // 5) El host registró exactamente una transacción.
    handle.shutdown();
    let report = timeout(WAIT, server)
        .await
        .expect("timeout esperando al mock")
        .expect("join")
        .expect("guion ok");
    assert_eq!(report.transactions, 1, "una transacción completa");
}
