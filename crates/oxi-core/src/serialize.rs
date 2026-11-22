//! Serialización de mensajes host→ATM (el espejo de `parse_host`).
//!
//! Necesaria para los fixtures y el host falso (fase 5): construir la
//! customización como tipos y emitirla al cable sin duplicar el formato.

use crate::command::TerminalCommand;
use crate::customisation::CustomisationData;
use crate::host::HostMessage;
use crate::reply::{InteractiveTransactionResponse, TransactionReply};
use crate::wire::{RawFields, FS};
use crate::ParseError;

impl HostMessage {
    /// Serializa el mensaje a los bytes del cable (sin encuadre de transporte).
    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        match self {
            Self::TerminalCommand(command) => serialize_command(command),
            Self::Customisation(data) => serialize_customisation(data),
            Self::TransactionReply(reply) => serialize_reply(reply),
            Self::InteractiveTransactionResponse(itr) => serialize_itr(itr),
            Self::EncryptionKeyChange { raw }
            | Self::ExtendedEncryptionKeyChange { raw }
            | Self::Unknown { raw } => Ok(raw.rebuild()),
        }
    }
}

/// Arma el encabezado común de los mensajes host→ATM: clase, LUNO vacío y
/// secuencia vacía (el driver real los manda vacíos y el terminal los ignora).
fn header(class: &str, payload: Vec<Vec<u8>>) -> Result<Vec<u8>, ParseError> {
    let mut fields = vec![class.as_bytes().to_vec(), Vec::new(), Vec::new()];
    fields.extend(payload);
    Ok(RawFields::new(fields)?.rebuild())
}

fn serialize_command(command: &TerminalCommand) -> Result<Vec<u8>, ParseError> {
    let body = match command {
        TerminalCommand::GoInService => "1".to_owned(),
        TerminalCommand::GoOutOfService { modifier } => format!("2{modifier}"),
        TerminalCommand::SendConfigurationId => "3".to_owned(),
        TerminalCommand::SendSupplyCounters => "4".to_owned(),
        TerminalCommand::SendTallyInformation { modifier } => format!("5{modifier}"),
        TerminalCommand::SendErrorLog => "6".to_owned(),
        TerminalCommand::SendConfigurationInformation { modifier } => format!("7{modifier}"),
        TerminalCommand::SendDateAndTime => "8".to_owned(),
        TerminalCommand::Disconnect => "F".to_owned(),
        TerminalCommand::NoOp => "G".to_owned(),
    };
    header("1", vec![body.into_bytes()])
}

fn serialize_customisation(data: &CustomisationData) -> Result<Vec<u8>, ParseError> {
    let (identifier, payload): (char, Vec<Vec<u8>>) = match data {
        CustomisationData::Screens(screens) => {
            let fields = screens
                .iter()
                .map(|screen| {
                    let mut field = screen.id.to_wire().into_bytes();
                    if !screen.keyboard.is_empty() {
                        field.push(crate::wire::GS);
                        field.extend_from_slice(&RawFields::join_groups(
                            &screen
                                .keyboard
                                .iter()
                                .map(Vec::as_slice)
                                .collect::<Vec<_>>(),
                        ));
                    } else {
                        field.extend_from_slice(&screen.data);
                    }
                    field
                })
                .collect();
            ('1', fields)
        }
        CustomisationData::StateTables(tables) => {
            let fields = tables
                .iter()
                .map(|table| {
                    let mut field = format!("{:03}", table.number.value()).into_bytes();
                    field.push(table.state_type as u8);
                    field.extend_from_slice(table.entries.as_bytes());
                    field
                })
                .collect();
            ('2', fields)
        }
        CustomisationData::ConfigParams(params) => {
            // Bloque fijo de 25: h(1)+i(3)+j(3)+k(3)+l(3)+m(3)+n(9).
            let head = format!(
                "0{:03}{:03}{:03}{:03}{:03}{:09}",
                "000",
                "000",
                "000",
                "000",
                params.options_mask % 1000,
                "000000000"
            );
            let mut fields = vec![head.into_bytes()];
            fields.push(params.luno.clone().unwrap_or_default().into_bytes());
            let timers: String = params
                .timers
                .iter()
                .map(|timer| format!("{:02}{:03}", timer.timer, timer.ticks))
                .collect();
            fields.push(timers.into_bytes());
            ('3', fields)
        }
        CustomisationData::Fits(fits) => {
            let fields = fits
                .iter()
                .map(|fit| {
                    let mut field = format!("{:03}", fit.number).into_bytes();
                    for entry in &fit.entries {
                        field.extend_from_slice(format!("{entry:03}").as_bytes());
                    }
                    field
                })
                .collect();
            ('5', fields)
        }
        CustomisationData::ConfigId(id) => ('6', vec![format!("{id:04}").into_bytes()]),
        CustomisationData::EnhancedConfig(ecpl) => {
            let mut fields = vec![ecpl.luno.clone().unwrap_or_default().into_bytes()];
            let options: String = ecpl
                .options
                .iter()
                .map(|option| format!("{:02}{:03}", option.number, option.code))
                .collect();
            fields.push(options.into_bytes());
            ('A', fields)
        }
        CustomisationData::MacFieldSelection(fields) => ('B', fields.clone()),
        CustomisationData::DateTime(dt) => {
            let value = if dt.quirk_14 {
                format!(
                    "{:04}{:02}{:02}{:02}{:02}00",
                    dt.year, dt.month, dt.day, dt.hour, dt.minute
                )
            } else {
                format!(
                    "{:02}{:02}{:02}{:02}{:02}",
                    dt.year % 100,
                    dt.month,
                    dt.day,
                    dt.hour,
                    dt.minute
                )
            };
            ('C', vec![value.into_bytes()])
        }
        CustomisationData::CassetteMapping(fields) => ('E', fields.clone()),
        CustomisationData::OverrideReservedScreens(fields) => ('G', fields.clone()),
    };
    header(
        "3",
        vec![format!("1{identifier}").into_bytes()]
            .into_iter()
            .chain(payload)
            .collect(),
    )
}

fn serialize_reply(reply: &TransactionReply) -> Result<Vec<u8>, ParseError> {
    let mut fields: Vec<Vec<u8>> = vec![
        b"4".to_vec(),
        Vec::new(),
        Vec::new(),
        format!("{:03}", reply.next_state.value()).into_bytes(),
        match reply.notes {
            Some(notes) => notes
                .counts()
                .iter()
                .map(|count| format!("{count:03}"))
                .collect::<String>()
                .into_bytes(),
            None => Vec::new(),
        },
        match (&reply.serial, reply.function_id, reply.screen) {
            (Some(serial), Some(function), Some(screen)) => {
                let mut field = format!("{serial}{function}{}", screen.to_wire()).into_bytes();
                if let Some(update) = &reply.screen_display_update {
                    field.extend_from_slice(update.as_bytes());
                }
                field
            }
            _ => Vec::new(),
        },
    ];
    let mut group_field = Vec::new();
    if let Some(mcn) = reply.mcn {
        group_field.push(mcn as u8);
    }
    if let Some(card) = reply.card_return {
        group_field.push(card.as_char() as u8);
    }
    if !reply.print_groups.is_empty() {
        let groups: Vec<Vec<u8>> = reply
            .print_groups
            .iter()
            .map(|group| {
                let mut bytes = vec![b'0' + group.flag.as_u8()];
                bytes.extend_from_slice(group.data.as_bytes());
                bytes
            })
            .collect();
        group_field.extend_from_slice(&RawFields::join_groups(
            &groups.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        ));
    }
    fields.push(group_field);
    // Campos extra crudos tal como se parsearon.
    for extra in &reply.extra {
        fields.push(extra.clone());
    }
    Ok(RawFields::new(fields)?.rebuild())
}

fn serialize_itr(itr: &InteractiveTransactionResponse) -> Result<Vec<u8>, ParseError> {
    let mut fields: Vec<Vec<u8>> = vec![
        b"3".to_vec(),
        Vec::new(),
        Vec::new(),
        format!("2{}{}", itr.display_flag, itr.active_keys).into_bytes(),
        itr.screen_timer.clone().unwrap_or_default().into_bytes(),
    ];
    if !itr.screen_data.is_empty() {
        fields.push(itr.screen_data.clone());
    }
    Ok(RawFields::new(fields)?.rebuild())
}

/// Separador de campo para tests de este módulo.
const _: u8 = FS;

#[cfg(test)]
mod tests {
    use crate::customisation::{Ecpl, EcplOption, FitDef, ScreenDef};
    use crate::types::{
        Fdk, Mcn, NoteCounts, PrintGroup, PrinterFlag, ScreenId, ScreenSelector, StateNumber,
    };

    use super::*;
    use crate::host::ParseOptions;

    fn roundtrip(message: HostMessage) {
        let bytes = message.to_bytes().expect("serialización válida");
        let raw = RawFields::parse(&bytes);
        let parsed = crate::parse_host(&raw, &ParseOptions::default()).expect("re-parse válido");
        assert_eq!(parsed, message, "roundtrip host→bytes→host");
    }

    #[test]
    fn command_should_roundtrip() {
        roundtrip(HostMessage::TerminalCommand(TerminalCommand::GoInService));
        roundtrip(HostMessage::TerminalCommand(
            TerminalCommand::GoOutOfService {
                modifier: "1".into(),
            },
        ));
        roundtrip(HostMessage::TerminalCommand(
            TerminalCommand::SendConfigurationInformation {
                modifier: "7".into(),
            },
        ));
    }

    #[test]
    fn screens_should_roundtrip() {
        roundtrip(HostMessage::Customisation(CustomisationData::Screens(
            vec![
                ScreenDef {
                    id: ScreenSelector::Customer(ScreenId::new(10).expect("en rango")),
                    data: b"\x0cBIENVENIDO".to_vec(),
                    keyboard: Vec::new(),
                },
                ScreenDef {
                    id: ScreenSelector::Group {
                        letter: 'C',
                        digits: 0,
                    },
                    data: b"\x0cOFFLINE".to_vec(),
                    keyboard: Vec::new(),
                },
            ],
        )));
    }

    #[test]
    fn state_tables_and_fits_should_roundtrip() {
        roundtrip(HostMessage::Customisation(CustomisationData::StateTables(
            vec![crate::StateTableDef {
                number: StateNumber::new(0).expect("en rango"),
                state_type: 'A',
                entries: "010001001002002002000001".into(),
            }],
        )));
        roundtrip(HostMessage::Customisation(CustomisationData::Fits(vec![
            FitDef {
                number: 1,
                entries: vec![0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00],
            },
        ])));
    }

    #[test]
    fn ecpl_and_config_id_should_roundtrip() {
        roundtrip(HostMessage::Customisation(
            CustomisationData::EnhancedConfig(Ecpl {
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
            }),
        ));
        roundtrip(HostMessage::Customisation(CustomisationData::ConfigId(1)));
    }

    #[test]
    fn date_time_should_roundtrip_in_both_formats() {
        let ten = crate::DateTimeLoad {
            year: 2025,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            quirk_14: false,
        };
        let bytes = HostMessage::Customisation(CustomisationData::DateTime(ten))
            .to_bytes()
            .expect("válido");
        assert!(bytes.windows(10).any(|window| window == b"2512312359"));
        roundtrip(HostMessage::Customisation(CustomisationData::DateTime(ten)));
    }

    #[test]
    fn reply_should_roundtrip_with_print_groups() {
        roundtrip(HostMessage::TransactionReply(TransactionReply {
            next_state: StateNumber::new(5).expect("en rango"),
            notes: NoteCounts::new([1, 0, 0, 0]).ok(),
            serial: Some("0001".into()),
            function_id: Some('2'),
            screen: Some(ScreenSelector::Customer(
                ScreenId::new(52).expect("en rango"),
            )),
            screen_display_update: None,
            mcn: Some('1'),
            card_return: Some(crate::CardReturnFlag::ReturnAtClose),
            print_groups: vec![PrintGroup {
                flag: PrinterFlag::Journal,
                data: "RECIBO".into(),
            }],
            extra: Vec::new(),
        }));
    }

    #[test]
    fn itr_should_roundtrip() {
        roundtrip(HostMessage::InteractiveTransactionResponse(
            InteractiveTransactionResponse {
                display_flag: '1',
                active_keys: "01".into(),
                screen_timer: Some("030".into()),
                screen_data: b"\x0cHOLA".to_vec(),
            },
        ));
    }

    #[test]
    fn mcn_should_survive_the_roundtrip() {
        let reply = TransactionReply {
            next_state: StateNumber::new(5).expect("en rango"),
            notes: None,
            serial: None,
            function_id: None,
            screen: None,
            screen_display_update: None,
            mcn: Some(Mcn::new('\u{31}', false).expect("MCN válido").as_char()),
            card_return: None,
            print_groups: Vec::new(),
            extra: Vec::new(),
        };
        roundtrip(HostMessage::TransactionReply(reply));
    }

    #[test]
    fn fdk_key_should_roundtrip_in_terminal_command() {
        let _ = Fdk::A; // el tipo de tecla no viaja en comandos host→ATM
    }
}
