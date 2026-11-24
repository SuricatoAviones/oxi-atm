//! Transaction Reply Command: clase 4 (SPEC §4.3; manual §10-58…§10-74).
//!
//! Layout del cuerpo verificado contra el parser de electron-atm que operó
//! contra el driver real del dueño:
//! `[4] [LUNO] [seq] [next_state] [notas] [serial+fn+pantalla] [MCN+flag+grupos]`
//! El campo de secuencia llega vacío y el terminal lo ignora (manual §10-58).

use crate::types::{
    CardReturnFlag, Fdk, NoteCounts, PrintGroup, PrinterFlag, ScreenSelector, StateNumber,
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

/// Interactive Transaction Response: clase 3, sub-clase 2 (manual §10-79).
///
/// Layout verificado contra el parser de electron-atm: `[3] "2"+flag+teclas`,
/// `[4] timer de pantalla`, `[5] datos de pantalla`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveTransactionResponse {
    /// Flag de display (primer char tras la sub-clase, §10-79).
    pub display_flag: char,
    /// Teclas activas como cadena binaria: el primer bit habilita el teclado
    /// numérico y los siguientes corresponden a las FDKs (§10-79). Formato
    /// DISTINTO de la máscara decimal de las state tables (SPEC §5.3).
    pub active_keys: String,
    /// Timer de pantalla en segundos (3 chars); `None` si no vino.
    pub screen_timer: Option<String>,
    /// Datos de pantalla dinámica (crudos, con caracteres de control).
    pub screen_data: Vec<u8>,
}

impl InteractiveTransactionResponse {
    /// Parsea el cuerpo de clase 3 sub-clase 2.
    pub(crate) fn parse(raw: &RawFields) -> Result<Self, ParseError> {
        let head = raw.ascii_field(3)?;
        let mut chars = head.chars();
        // El primer char es la sub-clase '2' (validada por el dispatcher).
        let _subclass = chars.next();
        let Some(display_flag) = chars.next() else {
            return Err(ParseError::malformed("ITR sin flag de display (§10-79)"));
        };
        if !matches!(display_flag, '0' | '1' | '2') {
            return Err(ParseError::invalid_field(
                "ITR display flag",
                format!("'{display_flag}': debe ser 0, 1 o 2 (§10-79)"),
            ));
        }
        let active_keys: String = chars.collect();
        if active_keys.is_empty() || active_keys.len() > 10 {
            return Err(ParseError::invalid_field(
                "ITR active keys",
                format!(
                    "'{active_keys}': longitud {} fuera de 1..=10 (§10-79)",
                    active_keys.len()
                ),
            ));
        }
        if !active_keys.chars().all(|c| c == '0' || c == '1') {
            return Err(ParseError::invalid_field(
                "ITR active keys",
                format!("'{active_keys}': se esperaba una cadena binaria (§10-79)"),
            ));
        }
        let screen_timer = match raw.fields().get(4) {
            Some(field) if field.is_empty() => None,
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("ITR timer no es ASCII"))?;
                if text.len() != 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::invalid_field(
                        "ITR screen timer",
                        format!("'{text}' debe ser 3 dígitos (§10-79)"),
                    ));
                }
                let ticks: u16 = text
                    .parse()
                    .map_err(|_| ParseError::invalid_field("ITR screen timer", text))?;
                if ticks > 255 {
                    return Err(ParseError::invalid_field(
                        "ITR screen timer",
                        format!("'{text}': rango 000-255 ticks (§10-79)"),
                    ));
                }
                Some(text.to_owned())
            }
            None => None,
        };
        let screen_data = match raw.fields().get(5) {
            Some(field) => field.clone(),
            None => Vec::new(),
        };
        Ok(Self {
            display_flag,
            active_keys,
            screen_timer,
            screen_data,
        })
    }

    /// ¿Está activo el teclado numérico completo? (campo h, char 1).
    pub fn numeric_active(&self) -> bool {
        self.key_active(0)
    }

    /// ¿Está activa esta FDK? (campo h; 'E' no participa de ITR, §10-79).
    pub fn fdk_active(&self, fdk: Fdk) -> bool {
        let index = match fdk {
            Fdk::A => 1,
            Fdk::B => 2,
            Fdk::C => 3,
            Fdk::D => 4,
            Fdk::E => return false,
            Fdk::F => 6,
            Fdk::G => 7,
            Fdk::H => 8,
            Fdk::I => 9,
        };
        self.key_active(index)
    }

    /// ¿Está activa la tecla Cancel? (campo h, char 6).
    pub fn cancel_active(&self) -> bool {
        self.key_active(5)
    }

    /// Enter está activo sólo si también lo está el teclado numérico y FDK A
    /// (Nota de §10-79).
    pub fn enter_active(&self) -> bool {
        self.numeric_active() && self.fdk_active(Fdk::A)
    }

    /// Ticks del timer de pantalla; `None` para 000 (indefinido).
    pub fn screen_timer_ticks(&self) -> Option<u16> {
        self.screen_timer
            .as_deref()
            .and_then(|text| text.parse::<u16>().ok())
            .filter(|ticks| *ticks > 0)
    }

    fn key_active(&self, index: usize) -> bool {
        self.active_keys.chars().nth(index) == Some('1')
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

    // ---- ITR (3/2) ----

    /// ITR sintética: display flag '1', teclado numérico + FDK A activos
    /// (cadena binaria: bit0 = numérico, bit1 = FDK A), timer 30 s, pantalla
    /// dinámica con controles.
    #[test]
    fn itr_should_parse_flag_keys_timer_and_screen() {
        let body = b"3\x1c\x1c\x1c210\x1c030\x1c\x0c\x0fGDHOLA";
        let itr = InteractiveTransactionResponse::parse(&raw(body)).expect("ITR válida");
        assert_eq!(itr.display_flag, '1');
        assert_eq!(itr.active_keys, "0");
        assert_eq!(itr.screen_timer.as_deref(), Some("030"));
        assert_eq!(itr.screen_data, b"\x0c\x0fGDHOLA");
    }

    #[test]
    fn itr_should_reject_non_binary_keys() {
        let body = b"3\x1c\x1c\x1c21X\x1c030";
        assert!(InteractiveTransactionResponse::parse(&raw(body)).is_err());
    }

    #[test]
    fn itr_should_reject_bad_timer() {
        let body = b"3\x1c\x1c\x1c2100\x1c3X";
        assert!(InteractiveTransactionResponse::parse(&raw(body)).is_err());
    }

    #[test]
    fn itr_should_reject_display_flag_outside_0_2() {
        let body = b"3\x1c\x1c\x1c2300\x1c030";
        assert!(InteractiveTransactionResponse::parse(&raw(body)).is_err());
    }

    #[test]
    fn itr_should_reject_empty_or_long_active_keys() {
        assert!(InteractiveTransactionResponse::parse(&raw(b"3\x1c\x1c\x1c21\x1c000")).is_err());
        let too_long = format!("3\x1c\x1c\x1c21{}\x1c000", "1".repeat(11));
        assert!(InteractiveTransactionResponse::parse(&raw(too_long.as_bytes())).is_err());
    }

    #[test]
    fn itr_should_reject_timer_above_255_ticks() {
        let body = b"3\x1c\x1c\x1c2100\x1c256";
        assert!(InteractiveTransactionResponse::parse(&raw(body)).is_err());
    }

    #[test]
    fn itr_helpers_should_map_active_keys() {
        // numérico + FDK A + FDK B + Cancel (posiciones 1, 2, 3 y 6).
        let body = format!("3\x1c\x1c\x1c21{}\x1c005", "1110010000");
        let itr = InteractiveTransactionResponse::parse(&raw(body.as_bytes())).expect("ITR válida");
        assert!(itr.numeric_active());
        assert!(itr.fdk_active(Fdk::A));
        assert!(itr.fdk_active(Fdk::B));
        assert!(!itr.fdk_active(Fdk::C));
        assert!(!itr.fdk_active(Fdk::E));
        assert!(itr.cancel_active());
        assert!(!itr.fdk_active(Fdk::I));
        assert!(itr.enter_active());
        assert_eq!(itr.screen_timer_ticks(), Some(5));
    }

    #[test]
    fn itr_timer_000_should_mean_indefinite() {
        let body = b"3\x1c\x1c\x1c2100\x1c000";
        let itr = InteractiveTransactionResponse::parse(&raw(body)).expect("ITR válida");
        assert_eq!(itr.screen_timer_ticks(), None);
    }
}
