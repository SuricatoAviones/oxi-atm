//! Fixtures sintéticas canónicas del repo: la ÚNICA fuente de customización
//! de test (AGENTS.md). Todo se construye con código propio; nada de datos
//! reales ni extractos del manual.
//!
//! El perfil reproduce el flujo del driver real (A→K→B→F→I→J) con estados y
//! pantallas mínimas, una FIT comodín y ECPL con opciones 12 y 15 activas
//! (specific reject y status data), igual que la descarga del dueño.

use oxi_core::{
    CardReturnFlag, CustomisationData, DateTimeLoad, Ecpl, EcplOption, FitDef, HostMessage,
    NoteCounts, PrintGroup, PrinterFlag, ScreenDef, ScreenId, ScreenSelector, StateNumber,
    StateTableDef, TerminalCommand, TransactionReply, TransactionRequest,
};

/// Pantallas del perfil (ids y textos sintéticos).
fn screens() -> Vec<ScreenDef> {
    let mut list = Vec::new();
    for (id, text) in [
        (10u16, "BIENVENIDO"),
        (20, "IMPORTE"),
        (23, "PROCESANDO"),
        (28, "PIN"),
        (52, "RECIBO"),
    ] {
        list.push(ScreenDef {
            id: ScreenSelector::Customer(ScreenId::new(id).expect("id sintético válido")),
            data: format!("\x0c{text}").into_bytes(),
            keyboard: Vec::new(),
        });
    }
    list
}

/// State tables del perfil (layouts del manual, valores sintéticos).
fn state_tables() -> Vec<StateTableDef> {
    let table = |number: u16, state_type: char, entries: &str| StateTableDef {
        number: StateNumber::new(number).expect("estado sintético válido"),
        state_type,
        entries: entries.into(),
    };
    vec![
        // A: pantalla 010, good 001, error 001, rc1/2/3 002 (pista 2),
        // devolver en close, no-FIT 001.
        table(0, 'A', "010001001002002002000001"),
        // K: salidas por índice de FIT (0 → 002 = PIN).
        table(1, 'K', "002002002002002002002002"),
        // B: pantalla 028, timeout 200, cancel 201, local 010/213/032,
        // remoto 003, reintentos 003.
        table(2, 'B', "028200201010213032003003"),
        // F: pantalla 020, timeout 200, cancel 200, FDK A 004, resto off.
        table(3, 'F', "020200200004255255255020"),
        // I: pantalla 023, timeout 203, pista 2, opcode, importe y PIN.
        table(4, 'I', "023203001000001001001000"),
        // J: recibo 052, next 000 (volver a A).
        table(5, 'J', "052000052052052052052"),
        // Cierres de error: pantalla 052 y next 255 (idle).
        table(200, 'J', "052255052052052052052"),
        table(201, 'J', "052255052052052052052"),
        table(203, 'J', "052255052052052052052"),
    ]
}

/// FIT comodín: número 001, PIDDX 000, PFIID FF×5 (cualquier tarjeta),
/// resto ceros (40 entradas: el formato del driver real, SPEC Q11).
fn fits() -> Vec<FitDef> {
    let mut entries = vec![0x00];
    entries.extend(std::iter::repeat_n(0xFF, 5));
    entries.extend(std::iter::repeat_n(0x00, 34));
    vec![FitDef { number: 1, entries }]
}

/// Secuencia de descarga completa (el "Download"), en orden.
pub fn customisation() -> Vec<HostMessage> {
    vec![
        HostMessage::Customisation(CustomisationData::Screens(screens())),
        HostMessage::Customisation(CustomisationData::StateTables(state_tables())),
        HostMessage::Customisation(CustomisationData::Fits(fits())),
        HostMessage::Customisation(CustomisationData::EnhancedConfig(Ecpl {
            luno: Some("001".into()),
            options: vec![
                EcplOption {
                    number: 12,
                    code: 1,
                },
                EcplOption {
                    number: 15,
                    code: 1,
                },
            ],
        })),
        HostMessage::Customisation(CustomisationData::DateTime(DateTimeLoad {
            year: 2025,
            month: 2,
            day: 22,
            hour: 11,
            minute: 39,
            quirk_14: false,
        })),
        HostMessage::Customisation(CustomisationData::ConfigId(1)),
    ]
}

/// Tarjeta de prueba sintética (pista 2 con FIID en el offset 0 de la FIT).
pub fn test_card() -> String {
    ";9999990000000000=2512?".into()
}

/// Transaction Reply de respuesta a un TRQ: aprueba (dispensa 1 billete y
/// devuelve tarjeta) o rechaza (deriva al cierre de error 200).
pub fn transaction_reply(request: &TransactionRequest, approve: bool) -> HostMessage {
    let base = TransactionReply {
        next_state: StateNumber::new(if approve { 5 } else { 200 }).expect("estado válido"),
        notes: approve.then(|| NoteCounts::new([1, 0, 0, 0]).expect("1 billete válido")),
        serial: Some("0001".into()),
        function_id: Some(if approve { '2' } else { '3' }),
        screen: Some(ScreenSelector::Customer(
            ScreenId::new(52).expect("pantalla válida"),
        )),
        screen_display_update: None,
        mcn: Some(request.mcn.as_char()),
        card_return: Some(CardReturnFlag::ReturnAtClose),
        print_groups: vec![PrintGroup {
            flag: PrinterFlag::Journal,
            data: if approve { "APROBADO" } else { "RECHAZADO" }.into(),
        }],
        extra: Vec::new(),
    };
    HostMessage::TransactionReply(base)
}

/// Comandos de la sesión de servicio.
pub fn go_in_service() -> HostMessage {
    HostMessage::TerminalCommand(TerminalCommand::GoInService)
}

/// Petición del Config ID (comando 3).
pub fn request_config_id() -> HostMessage {
    HostMessage::TerminalCommand(TerminalCommand::SendConfigurationId)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Todas las fixtures deben serializar al cable y re-parsear idénticas
    /// (los fixtures son también el test de la serialización de fase 5).
    #[test]
    fn all_fixtures_should_roundtrip() {
        let options = oxi_core::ParseOptions::default();
        for message in customisation() {
            let bytes = message.to_bytes().expect("fixture serializable");
            let parsed = oxi_core::parse_host(&oxi_core::RawFields::parse(&bytes), &options)
                .expect("fixture re-parseable");
            assert_eq!(parsed, message);
        }
    }

    #[test]
    fn reply_should_echo_the_request_mcn() {
        let request = TransactionRequest {
            luno: oxi_core::Luno::new("001").expect("válido"),
            tvn: None,
            top_of_receipt: '1',
            mcn: oxi_core::Mcn::new('\u{32}', false).expect("válido"),
            track2: None,
            track3: None,
            op_code: None,
            amount: None,
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: None,
            status_data: None,
        };
        let HostMessage::TransactionReply(reply) = transaction_reply(&request, true) else {
            panic!("debería ser un reply");
        };
        assert_eq!(reply.mcn, Some('\u{32}'));
    }
}
