//! Parseo de mensajes ATM→host (el espejo del serializador de `terminal`).
//!
//! Lo usa el host falso (fase 5) para interpretar TRQ y estados del ATM.

use crate::terminal::{StatusDescriptor, TerminalMessage, TransactionRequest};
use crate::types::{Luno, Mcn};
use crate::wire::RawFields;
use crate::ParseError;

impl StatusDescriptor {
    /// Descriptor desde el carácter del cable (§9-13).
    pub fn from_char(value: char) -> Result<Self, ParseError> {
        match value {
            '8' => Ok(Self::DeviceFault),
            '9' => Ok(Self::Ready),
            'A' => Ok(Self::CommandReject),
            'B' => Ok(Self::ReadyAlternate),
            'C' => Ok(Self::SpecificCommandReject),
            'F' => Ok(Self::TerminalState),
            other => Err(ParseError::invalid_field(
                "status descriptor",
                format!("'{other}' no documentado (§9-13)"),
            )),
        }
    }
}

impl TransactionRequest {
    /// Parsea un TRQ del cable (layout inverso de `to_bytes`, Tabla 9-1).
    fn from_fields(raw: &RawFields) -> Result<Self, ParseError> {
        let luno = Luno::new(raw.ascii_field(1)?)?;
        let tvn = optional(raw, 3)?;
        let flag_mcn = raw.ascii_field(4)?;
        let mut chars = flag_mcn.chars();
        let Some(top_of_receipt) = chars.next() else {
            return Err(ParseError::malformed("TRQ sin bandera de recibo/MCN"));
        };
        let Some(mcn_char) = chars.next() else {
            return Err(ParseError::malformed("TRQ sin MCN"));
        };
        // El parser acepta el rango extendido (opción 34) por tolerancia.
        let mcn = Mcn::new(mcn_char, true)?;

        let mut track1 = None;
        let mut status_data = None;
        let fields = raw.fields();
        let mut index = 12;
        while index < fields.len() {
            let Some(field) = fields.get(index) else {
                break;
            };
            match field.as_slice() {
                b"1" => {
                    track1 = optional(raw, index + 1)?;
                    index += 2;
                }
                b"2" => {
                    status_data = optional(raw, index + 1)?;
                    index += 2;
                }
                _ => index += 1,
            }
        }

        Ok(Self {
            luno,
            tvn,
            top_of_receipt,
            mcn,
            track2: optional(raw, 5)?,
            track3: optional(raw, 6)?,
            op_code: optional(raw, 7)?,
            amount: optional(raw, 8)?,
            buffer_a: optional(raw, 9)?,
            buffer_b: optional(raw, 10)?,
            buffer_c: optional(raw, 11)?,
            track1,
            status_data,
        })
    }
}

/// Campo `index` como `Some` si existe y no está vacío.
fn optional(raw: &RawFields, index: usize) -> Result<Option<String>, ParseError> {
    match raw.ascii_field(index) {
        Ok(text) if !text.is_empty() => Ok(Some(text.to_owned())),
        Ok(_) => Ok(None),
        Err(crate::ParseError::Malformed { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

/// Clasifica un mensaje del ATM (SPEC §3).
pub fn parse_terminal(raw: &RawFields) -> Result<TerminalMessage, ParseError> {
    let head = raw.ascii_field(0)?;
    let mut chars = head.chars();
    let Some(class) = chars.next() else {
        return Err(ParseError::malformed("mensaje del ATM sin clase"));
    };
    let subclass = chars.next();
    match (class, subclass) {
        ('1', Some('1')) => Ok(TerminalMessage::TransactionRequest(Box::new(
            TransactionRequest::from_fields(raw)?,
        ))),
        ('1', Some('2')) => {
            let luno = Luno::new(raw.ascii_field(1)?)?;
            // Tabla 9-24: "12" + LUNO + campo vacío + status information
            // (sin descriptor: los datos arrancan en el campo 3).
            let mut data: Vec<String> = raw
                .fields()
                .get(3..)
                .unwrap_or(&[])
                .iter()
                .map(|field| String::from_utf8_lossy(field).into_owned())
                .collect();
            if data.last().is_some_and(String::is_empty) {
                data.pop();
            }
            Ok(TerminalMessage::UnsolicitedStatus { luno, data })
        }
        ('2', Some('2')) => {
            let luno = Luno::new(raw.ascii_field(1)?)?;
            let descriptor_field = raw.ascii_field(3)?;
            let descriptor_char = descriptor_field
                .chars()
                .next()
                .ok_or_else(|| ParseError::malformed("status sin descriptor"))?;
            let descriptor = StatusDescriptor::from_char(descriptor_char)?;
            let mut data: Vec<String> = raw
                .fields()
                .get(4..)
                .unwrap_or(&[])
                .iter()
                .map(|field| String::from_utf8_lossy(field).into_owned())
                .collect();
            if data.last().is_some_and(String::is_empty) {
                data.pop();
            }
            Ok(TerminalMessage::SolicitedStatus {
                luno,
                descriptor,
                data,
            })
        }
        (class, subclass) => Err(ParseError::malformed(format!(
            "mensaje del ATM no soportado: {class}/{}",
            subclass.map(String::from).unwrap_or_default()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::StatusDescriptor;

    fn parse(bytes: &[u8]) -> TerminalMessage {
        parse_terminal(&RawFields::parse(bytes)).expect("mensaje del ATM válido")
    }

    /// Golden de la traza real del host: el Ready del electron-atm.
    #[test]
    fn ready_status_should_parse_from_trace_bytes() {
        let message = parse(b"22\x1c001\x1c\x1c9");
        assert_eq!(
            message,
            TerminalMessage::SolicitedStatus {
                luno: Luno::new("001").expect("válido"),
                descriptor: StatusDescriptor::Ready,
                data: Vec::new(),
            }
        );
    }

    /// Terminal State con datos (F + g1+subcampo).
    #[test]
    fn terminal_state_should_parse_data() {
        let message = parse(b"22\x1c001\x1c\x1cF\x1c60001");
        let TerminalMessage::SolicitedStatus {
            descriptor, data, ..
        } = message
        else {
            panic!("debería ser solicited");
        };
        assert_eq!(descriptor, StatusDescriptor::TerminalState);
        assert_eq!(data, vec!["60001".to_string()]);
    }

    #[test]
    fn unsolicited_should_parse() {
        let message = parse(b"12\x1c001\x1c\x1cB0001");
        let TerminalMessage::UnsolicitedStatus { data, .. } = message else {
            panic!("debería ser unsolicited");
        };
        assert_eq!(data, vec!["B0001".to_string()]);
    }

    /// Roundtrip completo: TRQ serializado → parse_terminal.
    #[test]
    fn request_should_roundtrip() {
        let original = TerminalMessage::TransactionRequest(Box::new(TransactionRequest {
            luno: Luno::new("001").expect("válido"),
            tvn: None,
            top_of_receipt: '0',
            mcn: Mcn::new('\u{31}', false).expect("MCN válido"),
            track2: Some(";5899410000000000=2512?".into()),
            track3: None,
            op_code: Some("AB      ".into()),
            amount: Some("00000100".into()),
            buffer_a: Some("0412A0EEDCBA9876".into()),
            buffer_b: None,
            buffer_c: None,
            track1: Some("B5899...".into()),
            status_data: Some("00011".into()),
        }));
        let bytes = original.to_bytes().expect("serialización válida");
        let parsed = parse_terminal(&RawFields::parse(&bytes)).expect("re-parse válido");
        assert_eq!(parsed, original);
    }

    #[test]
    fn unsupported_message_should_error() {
        let raw = RawFields::parse(b"51\x1cdatos");
        assert!(parse_terminal(&raw).is_err());
    }
}
