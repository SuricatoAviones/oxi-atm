//! Transaction Reply Command: clase 4 (SPEC §4.3; manual §10-58…§10-74).
//!
//! Layout del cuerpo verificado contra el parser de electron-atm que operó
//! contra el driver real del dueño:
//! `[4] [LUNO] [seq] [next_state] [notas] [serial+fn+pantalla] [MCN+flag+grupos]`
//! El campo de secuencia llega vacío y el terminal lo ignora (manual §10-58).

use crate::types::{
    CardReturnFlag, NoteCounts, PrintGroup, PrinterFlag, ScreenSelector, StateNumber,
};
use crate::wire::RawFields;
use crate::ParseError;

/// Transaction Reply del host (clase 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReply {
    /// Estado siguiente al que pasa el terminal (manual §10-58).
    pub next_state: StateNumber,
    /// Conteo de billetes por casete (4 × 3 dígitos; total ≤ 40, §10-58).
    /// `None` si el campo no vino.
    pub notes: Option<NoteCounts>,
    /// Serial de transacción (4 dígitos).
    pub serial: Option<String>,
    /// Function identifier (§10-60); crudo: el motor valida la tabla completa.
    pub function_id: Option<char>,
    /// Pantalla a mostrar tras el reply (3 chars).
    pub screen: Option<ScreenSelector>,
    /// Actualización de pantalla (resto del campo, opcional).
    pub screen_display_update: Option<String>,
    /// MCN devuelto por el host: debe cuadrar con el TRQ o ser '0' para
    /// anular la verificación (manual §9-2). Se guarda crudo porque '0'
    /// está fuera del rango de MCNs del terminal.
    pub mcn: Option<char>,
    /// Bandera de devolución de tarjeta (§10-59).
    pub card_return: Option<CardReturnFlag>,
    /// Grupos de impresión (≤ 13, GS-separados; §10-58).
    pub print_groups: Vec<PrintGroup>,
    /// Campos opcionales no modelados aún (escritura de pistas, datos VC,
    /// cheque, EMV — §10-58): preservados crudos para fases futuras.
    pub extra: Vec<Vec<u8>>,
}

impl TransactionReply {
    /// Parsea el cuerpo de clase 4.
    pub(crate) fn parse(raw: &RawFields) -> Result<Self, ParseError> {
        let fields = raw.fields();
        // El encabezado trae LUNO/secuencia opcionales que el terminal ignora
        // salvo el chequeo de número de máquina (manual §10-58); oxi v1 sin
        // data security los descarta.
        let next_text = raw.ascii_field(3)?;
        let next_state = Self::parse_next_state(next_text)?;

        let notes = match fields.get(4) {
            Some(field) if field.is_empty() => None,
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("notas no es ASCII"))?;
                if text.len() != 12 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::invalid_field(
                        "notes",
                        format!("'{text}': se esperaban 4 conteos de 3 dígitos (§10-58)"),
                    ));
                }
                let mut counts = [0u16; 4];
                for (i, chunk) in text.as_bytes().chunks_exact(3).enumerate() {
                    let value: u16 = std::str::from_utf8(chunk)
                        .expect("ya validado ASCII")
                        .parse()
                        .map_err(|_| ParseError::invalid_field("notes", "no numérico"))?;
                    counts[i] = value;
                }
                Some(NoteCounts::new(counts)?)
            }
            None => None,
        };

        let (serial, function_id, screen, screen_display_update) = match fields.get(5) {
            Some(field) if field.is_empty() => (None, None, None, None),
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("serial/función/pantalla no es ASCII"))?;
                if text.len() < 8 {
                    return Err(ParseError::malformed(format!(
                        "campo serial+función+pantalla incompleto: '{text}'"
                    )));
                }
                let serial: String = text[..4].to_owned();
                if !serial.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::invalid_field(
                        "transaction serial",
                        format!("'{serial}' debe ser 4 dígitos (§10-58)"),
                    ));
                }
                let function_id = text.chars().nth(4).expect("len >= 8");
                let (screen, rest) = ScreenSelector::take(&text[5..])?;
                let update = if rest.is_empty() {
                    None
                } else {
                    Some(rest.to_owned())
                };
                (Some(serial), Some(function_id), Some(screen), update)
            }
            None => (None, None, None, None),
        };

        let (mcn, card_return, print_groups) = match fields.get(6) {
            Some(field) if field.is_empty() => (None, None, Vec::new()),
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("MCN/tarjeta/impresión no es ASCII"))?;
                let mut chars = text.chars();
                let mcn = chars.next();
                let card_char = chars.next();
                let card_return = match card_char {
                    Some(c) => Some(CardReturnFlag::from_char(c)?),
                    None => None,
                };
                let rest: &str = chars.as_str();
                let mut print_groups = Vec::new();
                for group in RawFields::split_groups(rest.as_bytes()) {
                    if group.is_empty() {
                        continue;
                    }
                    // El flag viaja como dígito ASCII ('1' = 0x31), no como 1.
                    let flag_char = group[0];
                    if !flag_char.is_ascii_digit() {
                        return Err(ParseError::invalid_field(
                            "printer flag",
                            format!("'{}' no es un dígito (§10-58)", flag_char as char),
                        ));
                    }
                    let flag = PrinterFlag::from_u8(flag_char - b'0')?;
                    let data = String::from_utf8(group[1..].to_vec())
                        .map_err(|_| ParseError::malformed("datos de impresión no son ASCII"))?;
                    print_groups.push(PrintGroup { flag, data });
                }
                if print_groups.len() > 13 {
                    return Err(ParseError::invalid_field(
                        "print groups",
                        format!("{} > 13 grupos (§10-58/§10-87)", print_groups.len()),
                    ));
                }
                (mcn, card_return, print_groups)
            }
            None => (None, None, Vec::new()),
        };

        // Campos restantes (escritura de pistas, VC, cheque, EMV): crudos.
        let mut extra = Vec::new();
        if let Some(rest) = fields.get(7..) {
            extra.extend(rest.iter().cloned());
            if extra.last().is_some_and(Vec::is_empty) {
                extra.pop();
            }
        }

        Ok(Self {
            next_state,
            notes,
            serial,
            function_id,
            screen,
            screen_display_update,
            mcn,
            card_return,
            print_groups,
            extra,
        })
    }

    /// El estado siguiente usa el rango de las state tables (§10-9:
    /// 000–254 / 256–999; 255 no es un estado navegable).
    fn parse_next_state(text: &str) -> Result<StateNumber, ParseError> {
        if text.len() != 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError::invalid_field(
                "next state",
                format!("'{text}' debe ser 3 dígitos (§10-58)"),
            ));
        }
        let value: u16 = text
            .parse()
            .map_err(|_| ParseError::invalid_field("next state", "no numérico"))?;
        if value == 255 {
            return Err(ParseError::invalid_field(
                "next state",
                "255 no es un estado válido (§10-9)",
            ));
        }
        StateNumber::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(body: &[u8]) -> RawFields {
        RawFields::parse(body)
    }

    /// TR sintético de aprobación: next state 010, sin notas, serial 0001,
    /// función 2 (dispensar), pantalla 020, MCN 0, devolver en Close, un
    /// grupo de impresión de journal.
    #[test]
    fn reply_should_parse_next_state_function_screen_and_print() {
        let body = b"4\x1c\x1c\x1c010\x1c\x1c00012020\x1c001RECIBO";
        let reply = TransactionReply::parse(&raw(body)).expect("reply válido");
        assert_eq!(reply.next_state.value(), 10);
        assert_eq!(reply.serial.as_deref(), Some("0001"));
        assert_eq!(reply.function_id, Some('2'));
        assert_eq!(
            reply.screen,
            Some(ScreenSelector::Customer(
                crate::types::ScreenId::new(20).expect("en rango")
            ))
        );
        assert_eq!(reply.mcn, Some('0'));
        assert_eq!(reply.card_return, Some(CardReturnFlag::ReturnAtClose));
        assert_eq!(
            reply.print_groups,
            vec![PrintGroup {
                flag: PrinterFlag::Journal,
                data: "RECIBO".into()
            }]
        );
    }

    #[test]
    fn reply_should_parse_note_counts() {
        let body = b"4\x1c\x1c\x1c010\x1c005005005005";
        let reply = TransactionReply::parse(&raw(body)).expect("reply válido");
        let notes = reply.notes.expect("campo de notas presente");
        assert_eq!(notes.counts(), [5, 5, 5, 5]);
        assert_eq!(notes.total(), 20);
    }

    /// §10-87: dispensar más de 40 billetes se rechaza.
    #[test]
    fn reply_should_reject_notes_above_40() {
        let body = b"4\x1c\x1c\x1c010\x1c011011011011";
        assert!(TransactionReply::parse(&raw(body)).is_err());
    }

    #[test]
    fn reply_should_reject_13_plus_print_groups() {
        let groups = "\x1d1X".repeat(14);
        let body = format!("4\x1c\x1c\x1c010\x1c\x1c000128020\x1c01{groups}");
        assert!(TransactionReply::parse(&raw(body.as_bytes())).is_err());
    }

    #[test]
    fn reply_should_reject_next_state_255() {
        let body = b"4\x1c\x1c\x1c255";
        assert!(TransactionReply::parse(&raw(body)).is_err());
    }

    /// Reply mínimo: solo el next state; el resto de campos opcionales.
    #[test]
    fn reply_minimal_should_parse() {
        let body = b"4\x1c\x1c\x1c007";
        let reply = TransactionReply::parse(&raw(body)).expect("reply válido");
        assert_eq!(reply.next_state.value(), 7);
        assert_eq!(reply.notes, None);
        assert_eq!(reply.print_groups, Vec::<PrintGroup>::new());
    }
}
