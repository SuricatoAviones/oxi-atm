//! Tipos base del protocolo NDC con validación en construcción (SPEC §2, §5.1).
//!
//! Cada tipo rechaza en `new` lo que el manual declara inválido; los mensajes
//! malformados nunca llegan al estado del terminal.

use crate::ParseError;

/// Número de estado de una state table (SPEC §5.1; manual §2-1).
///
/// Rango válido: 000–254 y 256–999. El valor **255** existe en las tablas pero
/// significa "desactivado" (timer o tecla), no es un estado navegable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateNumber(u16);

/// Valor especial 255: desactiva la tecla/timer asociada (SPEC §5.1).
pub const STATE_DISABLED: u16 = 255;

impl StateNumber {
    /// Tope del rango de estados.
    pub const MAX: u16 = 999;

    /// Construye desde un decimal 000–999 (255 permitido como "disabled").
    pub fn new(value: u16) -> Result<Self, ParseError> {
        if value > Self::MAX {
            return Err(ParseError::OutOfRange {
                value: value.to_string(),
                expected: "000-254, 256-999 (255 = disabled)".into(),
            });
        }
        Ok(Self(value))
    }

    /// Valor decimal del estado.
    pub fn value(self) -> u16 {
        self.0
    }

    /// ¿Es el marcador 255 de desactivación?
    pub fn is_disabled(self) -> bool {
        self.0 == STATE_DISABLED
    }
}

impl std::fmt::Display for StateNumber {
    /// Serializa a 3 dígitos, como en las tablas descargadas.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:03}", self.0)
    }
}

/// Número lógico de terminal (LUNO), 3 dígitos (SPEC §2; manual §9-2).
///
/// El host lo fija vía ECPL (campo 'h', §10-21) — en el driver del dueño es el
/// código de cajero — y el terminal lo ecoa en TRQ y estados.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Luno(String);

impl Luno {
    /// Construye desde exactamente 3 dígitos.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        if value.len() != 3 || !value.chars().all(|c| c.is_ascii_digit()) {
            return Err(ParseError::invalid_field(
                "LUNO",
                format!("se esperaban 3 dígitos, llegó '{value}'"),
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Valor en texto (3 chars).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Número de coordinación de mensaje (MCN), asignado por el ATM a cada TRQ
/// (SPEC §2; manual §9-2).
///
/// Rango base 0x31–0x3F ('1'..'?'); con la opción 34 del ECPL se extiende a
/// 0x7E ('~') (manual §10-27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mcn(char);

impl Mcn {
    /// Primer MCN del ciclo.
    pub const FIRST: char = '\u{31}';

    /// Construye validando el rango (`extended` habilita el rango de opción 34).
    pub fn new(value: char, extended: bool) -> Result<Self, ParseError> {
        let top = if extended { '\u{7E}' } else { '\u{3F}' };
        if ('\u{31}'..=top).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ParseError::invalid_field(
                "MCN",
                format!("'{value}' fuera del rango 31h..{:02X}h", top as u8),
            ))
        }
    }

    /// Siguiente MCN del ciclo (rota al llegar al tope).
    pub fn next(self, extended: bool) -> Self {
        let top = if extended { '\u{7E}' } else { '\u{3F}' };
        let next = if self.0 >= top {
            Self::FIRST
        } else {
            char::from_u32(self.0 as u32 + 1).unwrap_or(Self::FIRST)
        };
        Self(next)
    }

    /// Carácter que viaja en el cable.
    pub fn as_char(self) -> char {
        self.0
    }
}

/// Tecla FDK del cardholder (manual §4-2: FDK 'A'–'I').
///
/// Nota: en las **máscaras** de state tables la tecla 'E' no participa
/// (SPEC §5.3, manual §2-61 Nota 18); ver [`FdkMask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fdk {
    /// FDK A (Enter en estados de importe).
    A,
    /// FDK B.
    B,
    /// FDK C.
    C,
    /// FDK D.
    D,
    /// FDK E (solo teclado, sin bit en máscaras).
    E,
    /// FDK F.
    F,
    /// FDK G.
    G,
    /// FDK H.
    H,
    /// FDK I.
    I,
}

impl Fdk {
    /// Letra de la tecla, tal como aparece en mensajes y tablas.
    pub fn as_char(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::D => 'D',
            Self::E => 'E',
            Self::F => 'F',
            Self::G => 'G',
            Self::H => 'H',
            Self::I => 'I',
        }
    }

    /// Letra → tecla.
    pub fn from_char(value: char) -> Result<Self, ParseError> {
        match value {
            'A' => Ok(Self::A),
            'B' => Ok(Self::B),
            'C' => Ok(Self::C),
            'D' => Ok(Self::D),
            'E' => Ok(Self::E),
            'F' => Ok(Self::F),
            'G' => Ok(Self::G),
            'H' => Ok(Self::H),
            'I' => Ok(Self::I),
            other => Err(ParseError::invalid_field(
                "FDK",
                format!("'{other}' no es una FDK válida (A-I)"),
            )),
        }
    }
}

/// Máscara de FDKs activas de una state table, 000–255 (SPEC §5.3).
///
/// Mapeo de bits (manual §2-61 Nota 18): bit 0='A', 1='B', 2='C', 3='D',
/// **4='F'**, 5='G', 6='H', 7='I'. La tecla 'E' no participa de la máscara
/// (el ejemplo del manual: 060 → C, D, F, G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FdkMask(u8);

impl FdkMask {
    /// Bit de cada tecla dentro de la máscara (None para 'E').
    fn bit_of(fdk: Fdk) -> Option<u8> {
        match fdk {
            Fdk::A => Some(0),
            Fdk::B => Some(1),
            Fdk::C => Some(2),
            Fdk::D => Some(3),
            Fdk::E => None,
            Fdk::F => Some(4),
            Fdk::G => Some(5),
            Fdk::H => Some(6),
            Fdk::I => Some(7),
        }
    }

    /// Máscara desde el byte crudo (000-255 del campo de tabla).
    pub fn new(raw: u8) -> Self {
        Self(raw)
    }

    /// Byte crudo de la máscara.
    pub fn raw(self) -> u8 {
        self.0
    }

    /// ¿Está la tecla activa? ('E' siempre false: no tiene bit).
    pub fn is_active(self, fdk: Fdk) -> bool {
        match Self::bit_of(fdk) {
            Some(bit) => self.0 & (1 << bit) != 0,
            None => false,
        }
    }

    /// Teclas activas, en orden A..I.
    pub fn active_fdks(self) -> impl Iterator<Item = Fdk> {
        [
            Fdk::A,
            Fdk::B,
            Fdk::C,
            Fdk::D,
            Fdk::E,
            Fdk::F,
            Fdk::G,
            Fdk::H,
            Fdk::I,
        ]
        .into_iter()
        .filter(move |f| self.is_active(*f))
    }
}

/// Conteo de billetes por casete del Transaction Reply (SPEC §4.3).
///
/// Cuatro campos de 3 dígitos; el total no puede exceder 40 billetes
/// (manual §10-58).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteCounts([u16; 4]);

impl NoteCounts {
    /// Máximo total de billetes por Transaction Reply.
    pub const MAX_TOTAL: u16 = 40;

    /// Construye desde los 4 conteos, validando el total.
    pub fn new(counts: [u16; 4]) -> Result<Self, ParseError> {
        let total: u16 = counts.iter().sum();
        if counts.iter().any(|&c| c > 999) {
            return Err(ParseError::OutOfRange {
                value: format!("{counts:?}"),
                expected: "conteos de 3 dígitos (000-999)".into(),
            });
        }
        if total > Self::MAX_TOTAL {
            return Err(ParseError::invalid_field(
                "notes",
                format!(
                    "total {total} excede el máximo de {} billetes",
                    Self::MAX_TOTAL
                ),
            ));
        }
        Ok(Self(counts))
    }

    /// Los 4 conteos.
    pub fn counts(self) -> [u16; 4] {
        self.0
    }

    /// Total de billetes.
    pub fn total(self) -> u16 {
        self.0.iter().sum()
    }
}

/// Identificador de pantalla (SPEC §6; manual §3-1).
///
/// Rango 0000–9999; el cable admite 3 dígitos por compatibilidad (el driver
/// real del dueño usa 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenId(u16);

impl ScreenId {
    /// Tope del rango.
    pub const MAX: u16 = 9999;

    /// Construye desde un decimal 0–9999.
    pub fn new(value: u16) -> Result<Self, ParseError> {
        if value > Self::MAX {
            return Err(ParseError::OutOfRange {
                value: value.to_string(),
                expected: "0000-9999".into(),
            });
        }
        Ok(Self(value))
    }

    /// Valor decimal.
    pub fn value(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for ScreenId {
    /// Serializa a 4 dígitos (formato principal del manual).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}", self.0)
    }
}

/// Identificador de pantalla (SPEC §6; manual §10-11 campo 'h': 3, 5 o 6 chars).
///
/// Formas del cable:
/// - 3 dígitos (pantalla de cardholder, compatibilidad);
/// - `"0123"`: 4 dígitos entre comillas;
/// - letra de grupo + 2 dígitos (`C04`) o letra + 4 dígitos (`E0123`, solo
///   grupos E/X/Y/Z/u; la 'l' mapea a 'u', manual §10-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenSelector {
    /// Pantalla de cardholder (numérica, 0–9999).
    Customer(ScreenId),
    /// Pantalla de grupo/reservada: letra + número.
    Group {
        /// Letra de grupo ('C', 'E', 'u', …).
        letter: char,
        /// Número dentro del grupo (2 o 4 dígitos).
        digits: u16,
    },
}

impl ScreenSelector {
    /// Extrae el id del inicio de un campo de pantalla, devolviendo el resto
    /// (datos de pantalla, posiblemente con teclado tras un GS).
    pub fn take(text: &str) -> Result<(Self, &str), ParseError> {
        let mut chars = text.chars();
        match chars.next() {
            // "0123": 4 dígitos entre comillas (6 chars en total).
            Some('"') => {
                let digits: String = text.chars().skip(1).take(4).collect();
                let closing = text.chars().nth(5);
                if digits.len() == 4
                    && digits.bytes().all(|b| b.is_ascii_digit())
                    && closing == Some('"')
                {
                    let value: u16 = digits
                        .parse()
                        .map_err(|_| ParseError::invalid_field("screen number", "no numérico"))?;
                    let id = ScreenId::new(value)?;
                    Ok((Self::Customer(id), &text[6..]))
                } else {
                    Err(ParseError::invalid_field(
                        "screen number",
                        format!("'{text}': comillas sin 4 dígitos (manual §10-11)"),
                    ))
                }
            }
            // Letra de grupo: E/X/Y/Z/u/l llevan 4 dígitos; el resto, 2.
            Some(letter) if letter.is_ascii_alphabetic() => {
                let digit_count = if matches!(letter, 'E' | 'X' | 'Y' | 'Z' | 'u' | 'l') {
                    4
                } else {
                    2
                };
                let digits: String = text.chars().skip(1).take(digit_count).collect();
                if digits.len() == digit_count && digits.bytes().all(|b| b.is_ascii_digit()) {
                    let value: u16 = digits.parse().map_err(|_| {
                        ParseError::invalid_field("screen group number", "no numérico")
                    })?;
                    // 'l' mapea directamente a 'u' (manual §10-11).
                    let letter = if letter == 'l' { 'u' } else { letter };
                    Ok((
                        Self::Group {
                            letter,
                            digits: value,
                        },
                        &text[1 + digit_count..],
                    ))
                } else {
                    Err(ParseError::invalid_field(
                        "screen number",
                        format!("'{text}': grupo sin {digit_count} dígitos (manual §10-11)"),
                    ))
                }
            }
            // 3 dígitos sueltos.
            Some(_) => {
                let digits: String = text.chars().take(3).collect();
                if digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()) {
                    let value: u16 = digits
                        .parse()
                        .map_err(|_| ParseError::invalid_field("screen number", "no numérico"))?;
                    let id = ScreenId::new(value)?;
                    Ok((Self::Customer(id), &text[3..]))
                } else {
                    Err(ParseError::invalid_field(
                        "screen number",
                        format!("'{text}': se esperaban 3 dígitos (manual §10-11)"),
                    ))
                }
            }
            None => Err(ParseError::malformed("campo de pantalla vacío")),
        }
    }
}

/// Bandera de devolución de tarjeta del Transaction Reply (SPEC §4.3).
///
/// Valores documentados (manual §10-59): '0' devolver en el Close,
/// '1' retener, '4' devolver durante el procesamiento del reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardReturnFlag {
    /// '0': devolver la tarjeta en el estado Close.
    ReturnAtClose,
    /// '1': retener la tarjeta.
    Retain,
    /// '4': devolver durante el procesamiento del Transaction Reply.
    ReturnDuringReply,
}

impl CardReturnFlag {
    /// Char → bandera. Otros valores se rechazan aquí y T1.5 decide si el
    /// mensaje completo se rechaza o se aplica default del manual.
    pub fn from_char(value: char) -> Result<Self, ParseError> {
        match value {
            '0' => Ok(Self::ReturnAtClose),
            '1' => Ok(Self::Retain),
            '4' => Ok(Self::ReturnDuringReply),
            other => Err(ParseError::invalid_field(
                "card return flag",
                format!("'{other}' no documentado (0/1/4, manual §10-59)"),
            )),
        }
    }

    /// Char que viaja en el cable.
    pub fn as_char(self) -> char {
        match self {
            Self::ReturnAtClose => '0',
            Self::Retain => '1',
            Self::ReturnDuringReply => '4',
        }
    }
}

/// Flag de impresión de un grupo de impresión del Transaction Reply (SPEC §4.3).
///
/// Valores base documentados y verificados contra el driver real: 0 no
/// imprimir, 1 solo journal, 2 solo receipt, 3 ambos. Las variantes PPD /
/// statement / sideways se mapean cuando T1.5 verifique la tabla §10-58.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterFlag {
    /// 0: no imprimir.
    NoPrint,
    /// 1: imprimir en journal.
    Journal,
    /// 2: imprimir en receipt.
    Receipt,
    /// 3: imprimir en journal y receipt.
    JournalAndReceipt,
}

impl PrinterFlag {
    /// Byte → flag. Los valores no documentados aún devuelven error y se
    /// incorporan en T1.5 con la tabla completa de §10-58.
    pub fn from_u8(value: u8) -> Result<Self, ParseError> {
        match value {
            0 => Ok(Self::NoPrint),
            1 => Ok(Self::Journal),
            2 => Ok(Self::Receipt),
            3 => Ok(Self::JournalAndReceipt),
            other => Err(ParseError::invalid_field(
                "printer flag",
                format!("'{other}' se mapea en T1.5 con la tabla §10-58"),
            )),
        }
    }

    /// Byte que viaja en el cable.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::NoPrint => 0,
            Self::Journal => 1,
            Self::Receipt => 2,
            Self::JournalAndReceipt => 3,
        }
    }
}

/// Un grupo de impresión del Transaction Reply: flag + datos (SPEC §4.3).
///
/// Los grupos van separados por GS (0x1D) dentro de su campo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintGroup {
    /// Destino de la impresión.
    pub flag: PrinterFlag,
    /// Datos de impresión (controles de impresora incluidos).
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_number_should_accept_zero() {
        assert!(StateNumber::new(0).is_ok());
    }

    #[test]
    fn state_number_should_accept_254() {
        assert!(StateNumber::new(254).is_ok());
    }

    #[test]
    fn state_number_should_accept_256() {
        assert!(StateNumber::new(256).is_ok());
    }

    #[test]
    fn state_number_should_accept_999() {
        assert!(StateNumber::new(999).is_ok());
    }

    #[test]
    fn state_number_should_reject_1000() {
        assert!(StateNumber::new(1000).is_err());
    }

    #[test]
    fn state_number_255_should_be_disabled_marker() {
        let state = StateNumber::new(255).expect("255 es válido como marcador");
        assert!(state.is_disabled());
    }

    #[test]
    fn state_number_should_display_three_digits() {
        let state = StateNumber::new(7).expect("7 en rango");
        assert_eq!(state.to_string(), "007");
    }

    #[test]
    fn luno_should_accept_three_digits() {
        assert!(Luno::new("001").is_ok());
    }

    #[test]
    fn luno_should_reject_short_value() {
        assert!(Luno::new("1").is_err());
    }

    #[test]
    fn luno_should_reject_non_digits() {
        assert!(Luno::new("01A").is_err());
    }

    #[test]
    fn mcn_should_accept_first_of_base_range() {
        assert!(Mcn::new('\u{31}', false).is_ok());
    }

    #[test]
    fn mcn_should_accept_top_of_base_range() {
        assert!(Mcn::new('\u{3F}', false).is_ok());
    }

    #[test]
    fn mcn_should_reject_value_above_base_range_without_option_34() {
        assert!(Mcn::new('\u{40}', false).is_err());
    }

    #[test]
    fn mcn_should_accept_extended_value_with_option_34() {
        assert!(Mcn::new('\u{40}', true).is_ok());
    }

    #[test]
    fn mcn_next_should_rotate_at_base_top() {
        let mcn = Mcn::new('\u{3F}', false).expect("tope del rango base");
        assert_eq!(mcn.next(false).as_char(), '\u{31}');
    }

    #[test]
    fn fdk_from_char_should_reject_out_of_range() {
        assert!(Fdk::from_char('J').is_err());
    }

    /// Caso del manual (§2-61 Nota 18): máscara 060 → C, D, F, G.
    #[test]
    fn fdk_mask_060_should_activate_c_d_f_g() {
        let mask = FdkMask::new(60);
        let active: Vec<char> = mask.active_fdks().map(Fdk::as_char).collect();
        assert_eq!(active, vec!['C', 'D', 'F', 'G']);
    }

    /// La tecla 'E' no tiene bit en la máscara (SPEC §5.3).
    #[test]
    fn fdk_mask_should_never_activate_e() {
        let all = FdkMask::new(0xFF);
        assert!(!all.is_active(Fdk::E));
    }

    #[test]
    fn note_counts_should_reject_total_above_40() {
        assert!(NoteCounts::new([10, 10, 10, 11]).is_err());
    }

    #[test]
    fn note_counts_should_accept_total_40() {
        assert!(NoteCounts::new([10, 10, 10, 10]).is_ok());
    }

    #[test]
    fn screen_id_should_reject_above_9999() {
        assert!(ScreenId::new(10_000).is_err());
    }

    #[test]
    fn screen_id_should_display_four_digits() {
        let screen = ScreenId::new(15).expect("15 en rango");
        assert_eq!(screen.to_string(), "0015");
    }

    #[test]
    fn card_return_flag_should_reject_undocumented_value() {
        assert!(CardReturnFlag::from_char('2').is_err());
    }

    #[test]
    fn printer_flag_should_reject_value_pending_t1_5() {
        assert!(PrinterFlag::from_u8(4).is_err());
    }
}
