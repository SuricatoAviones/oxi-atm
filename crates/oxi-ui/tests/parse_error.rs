//! T8.2: un frame malformado del host debe generar un reject, no silencio.

use std::time::Duration;

use oxi_core::{parse_terminal, RawFields, StatusDescriptor, TerminalMessage};
use oxi_host_mock::{read_frame, write_frame};
use oxi_ui::{spawn, SimConfig};
use tokio::net::TcpListener;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread")]
async fn malformed_host_frame_should_get_a_reject() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("puerto efímero");
    let addr = listener.local_addr().expect("addr");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("ATM conecta");
        // Date/Time con 14 chars: el parser estricto lo rechaza (Q12).
        write_frame(&mut stream, b"3\x1c\x1c\x1c1C\x1c20251231235959")
            .await
            .expect("frame malformado");
        read_frame(&mut stream)
            .await
            .expect("lectura del reject")
            .expect("frame de respuesta")
    });

    let mut config = SimConfig::default();
    config.net.host = "127.0.0.1".into();
    config.net.port = addr.port();
    config.net.max_attempts = 1;
    let handle = spawn(&config);

    let frame = timeout(WAIT, server)
        .await
        .expect("timeout esperando el reject")
        .expect("task del servidor");
    let message = parse_terminal(&RawFields::parse(&frame)).expect("status válido");
    match message {
        TerminalMessage::SolicitedStatus { descriptor, .. } => {
            assert!(
                matches!(
                    descriptor,
                    StatusDescriptor::CommandReject | StatusDescriptor::SpecificCommandReject
                ),
                "se esperaba reject, llegó {descriptor:?}"
            );
        }
        other => panic!("se esperaba Solicited Status, llegó {other:?}"),
    }
    handle.shutdown();
}
