//! Customisation Data Commands: clase 3, sub-clase 1 (SPEC §4.2).
//!
//! Identificadores parseados aquí (manual §10-7, Tabla 10-2): '1' pantallas,
//! '2' state tables, '3' CPL, '5' FITs, '6' Config ID, 'A' ECPL, 'B' selección
//! MAC, 'C' fecha/hora, 'E' mapeo de casetes, 'G' override de pantallas
//! reservadas.
//!
//! Formatos verificados contra el download real del dueño (nov-2026): número y
//! datos de estado/FIT van **concatenados** en un campo FS (manual §10-9/§10-31
//! no ponen separador entre h e i); las pantallas van en campos alternados
//! id/datos; los mensajes suelen terminar con FS, lo que deja un campo vacío
//! final que aquí se tolera.

use crate::host::ParseOptions;
use crate::types::{ScreenSelector, StateNumber};
use crate::wire::{RawFields, GS};
use crate::ParseError;

/// Entradas máximas por FIT (manual §10-31: el terminal almacena 41; el
/// exceso se ignora). El driver real envía 40 (SPEC §14 Q11).
const FIT_MAX_ENTRIES: usize = 41;

/// Tipos de estado legales en la descarga (manual §10-9, campo i). Sin 'O' ni
/// 'U' mayúsculas; minúsculas solo b/d/e/f/g/i/k/w, más '_' y '>'.
const LEGAL_STATE_TYPES: &str = "ABCDEFGHIJKLMNPQRSTVWXYZbdefgikw_>";

/// Un comando de customización (clase 3/1) ya tipado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomisationData {
    /// '1': Screen/Keyboard Data Load (manual §10-11).
    Screens(Vec<ScreenDef>),
    /// '2': State Tables Load (manual §10-9).
    StateTables(Vec<StateTableDef>),
    /// '3': Configuration Parameters Load (herencia Diebold, manual §10-14).
    ConfigParams(ConfigParams),
    /// '5': FIT Data Load (manual §10-31).
    Fits(Vec<FitDef>),
    /// '6': Configuration ID Number Load (manual §10-33, 0001-9999).
    ConfigId(u16),
    /// 'A': Enhanced Configuration Parameters Load (manual §10-21).
    EnhancedConfig(Ecpl),
    /// 'B': MAC Field Selection Load (manual §10-35). Crudo en v1 (sin MAC).
    MacFieldSelection(Vec<Vec<u8>>),
    /// 'C': Date and Time Load (manual §10-41) — decisión Q12 del SPEC.
    DateTime(DateTimeLoad),
    /// 'E': Dispenser Currency Cassette Mapping Table (manual §10-51). Crudo
    /// en v1; se tipa en la fase de hardware.
    CassetteMapping(Vec<Vec<u8>>),
    /// 'G': Override Reserved Screens (manual §10-55). Crudo en v1.
    OverrideReservedScreens(Vec<Vec<u8>>),
}

/// Una pantalla descargada: id + datos (+ teclado touch opcional tras GS).
///
/// Estructura del campo según manual §10-11: número de pantalla (3, 5 o 6
/// chars, ver [`ScreenSelector`]) **concatenado** con los datos; si hay datos
/// de teclado, un GS los separa (Tabla Nota 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenDef {
    /// Identificador de pantalla.
    pub id: ScreenSelector,
    /// Datos crudos de pantalla (controles FF/SI/ESC/SO incluidos, manual §3-14).
    pub data: Vec<u8>,
    /// Datos de teclado (touch) tras el GS — crudos en v1 (Tabla Nota 2).
    pub keyboard: Vec<Vec<u8>>,
}

/// Una state table descargada (manual §10-9: número + tipo + entradas).
///
/// Las entradas se guardan crudas (alineadas de a 3 chars); la semántica por
/// tipo es trabajo del intérprete de la fase 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTableDef {
    /// Número 000–254 / 256–999 (el 255 NO es un estado válido, §10-9).
    pub number: StateNumber,
    /// Letra de tipo legal según §10-9.
    pub state_type: char,
    /// Entradas crudas tras la letra de tipo (longuencia múltiplo de 3).
    pub entries: String,
}

/// Configuration Parameters Load (manual §10-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParams {
    /// Campo 'm': bitmask 001 ready / 002 supply / 008 buffer 12 dígitos
    /// (se suman; manual §10-14). Los campos h–l no soportados se ignoran.
    pub options_mask: u16,
    /// LUNO del terminal (campo 'o'); None si no vino.
    pub luno: Option<String>,
    /// Pares timer/ticks de 800 ms (campos p/q repetibles).
    pub timers: Vec<TimerEntry>,
}

/// Un par timer/ticks del CPL (manual §10-14: ticks de 800 ms, 000 = infinito).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerEntry {
    /// Número de timer (00–99).
    pub timer: u8,
    /// Ticks de 800 ms (000–255).
    pub ticks: u8,
}

/// Una FIT descargada (manual §10-31): número + entradas decodificadas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FitDef {
    /// Número 000–999: define el orden de búsqueda (§10-31).
    pub number: u16,
    /// Entradas decodificadas a bytes (≤ 41; el exceso se ignora, §10-31).
    /// Campos omitidos valen 0 (manual §8-2).
    pub entries: Vec<u8>,
}

/// Enhanced Configuration Parameters Load (manual §10-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ecpl {
    /// LUNO del terminal (campo 'h'); el driver real inserta el código de
    /// cajero aquí (verificado en runtime); vacío → None.
    pub luno: Option<String>,
    /// Pares opción/código (campos i/j repetibles, 2+3 chars).
    pub options: Vec<EcplOption>,
}

/// Un par opción/código del ECPL (manual §10-22…§10-28).
///
/// Los números de opción no documentados (reservados) se preservan: el
/// motor los ignora con warning (SPEC §14 Q13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcplOption {
    /// Número de opción (00–99).
    pub number: u8,
    /// Código (000–999).
    pub code: u16,
}

/// Date and Time Load (manual §10-41) con la decisión Q12 del SPEC.
///
/// - Formato del manual: 10 chars `YYMMDDHHMM` (año: 00–89 → 2000–2089,
///   90–99 → 1990–1999; segundos en cero).
/// - Divergencia del driver actual del dueño: 14 chars `yyyyMMddHHmmss`.
///   Se acepta **solo** con `tolerate_driver_quirks` (SPEC §14 Q12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeLoad {
    /// Año completo (p.ej. 2025).
    pub year: u16,
    /// Mes 1–12.
    pub month: u8,
    /// Día 1–31.
    pub day: u8,
    /// Hora 0–23.
    pub hour: u8,
    /// Minuto 0–59.
    pub minute: u8,
    /// true si vino en el formato de 14 chars del driver (solo posible con
    /// `tolerate_driver_quirks`); para diagnóstico y logs.
    pub quirk_14: bool,
}

impl CustomisationData {
    /// Parsea según el identificador (2º char del campo de cabecera).
    ///
    /// `None` = identificador no documentado en el manual → el llamador lo
    /// envuelve en `HostMessage::Unknown`.
    pub(crate) fn parse(
        raw: &RawFields,
        identifier: char,
        options: &ParseOptions,
    ) -> Result<Option<Self>, ParseError> {
        match identifier {
            '1' => Self::parse_screens(raw).map(|v| Some(Self::Screens(v))),
            '2' => Self::parse_state_tables(raw).map(|v| Some(Self::StateTables(v))),
            '3' => Self::parse_config_params(raw).map(|v| Some(Self::ConfigParams(v))),
            '5' => Self::parse_fits(raw).map(|v| Some(Self::Fits(v))),
            '6' => Self::parse_config_id(raw).map(|v| Some(Self::ConfigId(v))),
            'A' => Self::parse_enhanced_config(raw).map(|v| Some(Self::EnhancedConfig(v))),
            'B' => Ok(Some(Self::MacFieldSelection(Self::raw_payload(raw)))),
            'C' => Self::parse_date_time(raw, options).map(|v| Some(Self::DateTime(v))),
            'E' => Ok(Some(Self::CassetteMapping(Self::raw_payload(raw)))),
            'G' => Ok(Some(Self::OverrideReservedScreens(Self::raw_payload(raw)))),
            _ => Ok(None),
        }
    }

    /// Campos de payload desde el índice 4, tolerando el campo vacío final
    /// que deja el FS de cierre (patrón verificado en el driver real).
    fn raw_payload(raw: &RawFields) -> Vec<Vec<u8>> {
        let fields = Self::payload_fields(raw);
        fields.to_vec()
    }

    fn payload_fields(raw: &RawFields) -> &[Vec<u8>] {
        let mut fields = raw.fields().get(4..).unwrap_or(&[]);
        if fields.last().is_some_and(Vec::is_empty) {
            fields = &fields[..fields.len() - 1];
        }
        fields
    }

    /// '1': cada campo = número de pantalla concatenado con sus datos
    /// (manual §10-11: h + i sin FS entre ellos), con teclado opcional tras
    /// un GS (Tabla Nota 2). Patrón verificado contra el download real.
    fn parse_screens(raw: &RawFields) -> Result<Vec<ScreenDef>, ParseError> {
        let mut screens = Vec::new();
        for field in Self::payload_fields(raw) {
            let text = std::str::from_utf8(field)
                .map_err(|_| ParseError::malformed("pantalla no es ASCII"))?;
            let (id, rest) = ScreenSelector::take(text)?;
            // GS separa el teclado del data de pantalla (§10-11 Tabla Nota 2).
            let (data, keyboard) = match rest.as_bytes().iter().position(|&b| b == GS) {
                Some(index) => (
                    rest.as_bytes()[..index].to_vec(),
                    RawFields::split_groups(&rest.as_bytes()[index + 1..])
                        .into_iter()
                        .map(<[u8]>::to_vec)
                        .collect(),
                ),
                None => (rest.as_bytes().to_vec(), Vec::new()),
            };
            screens.push(ScreenDef { id, data, keyboard });
        }
        Ok(screens)
    }

    /// '2': cada campo = número(3) + tipo(1) + entradas (múltiplo de 3).
    fn parse_state_tables(raw: &RawFields) -> Result<Vec<StateTableDef>, ParseError> {
        let mut tables = Vec::new();
        for field in Self::payload_fields(raw) {
            let text = std::str::from_utf8(field)
                .map_err(|_| ParseError::malformed("state table no es ASCII"))?;
            if text.len() < 4 {
                return Err(ParseError::malformed(format!(
                    "state table sin datos: '{text}'"
                )));
            }
            let (number_str, data) = text.split_at(3);
            let number: u16 = parse_digits(number_str, "state number")?;
            // §10-9: rango 000-254 o 256-999; 255 no es número de estado.
            if number == 255 {
                return Err(ParseError::invalid_field(
                    "state number",
                    "255 no es un número de estado válido (manual §10-9)",
                ));
            }
            let table_number = StateNumber::new(number)?;
            let state_type = data
                .chars()
                .next()
                .ok_or_else(|| ParseError::malformed("state table sin letra de tipo"))?;
            if !LEGAL_STATE_TYPES.contains(state_type) {
                return Err(ParseError::invalid_field(
                    "state type",
                    format!("'{state_type}' no es un tipo legal (manual §10-9); "),
                ));
            }
            let entries = &data[1..];
            if entries.len() % 3 != 0 {
                return Err(ParseError::malformed(format!(
                    "entradas de la state table {table_number} no alineadas a 3 chars"
                )));
            }
            tables.push(StateTableDef {
                number: table_number,
                state_type,
                entries: entries.to_owned(),
            });
        }
        Ok(tables)
    }

    /// '3': bloque fijo de 25 chars + LUNO opcional + pares timer/ticks
    /// (manual §10-14; los campos h–l no soportados se ignoran).
    fn parse_config_params(raw: &RawFields) -> Result<ConfigParams, ParseError> {
        let head = raw
            .fields()
            .get(4)
            .ok_or_else(|| ParseError::malformed("CPL sin bloque de parámetros"))?;
        let head_str =
            std::str::from_utf8(head).map_err(|_| ParseError::malformed("CPL no es ASCII"))?;
        if head_str.len() < 25 || !head_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError::malformed(
                "CPL: se esperaba el bloque fijo de 25 dígitos (manual §10-14)",
            ));
        }
        let options_mask: u16 = head_str[13..16]
            .parse()
            .map_err(|_| ParseError::invalid_field("CPL options", "campo 'm' no numérico"))?;
        if options_mask > 255 {
            return Err(ParseError::invalid_field(
                "CPL options",
                format!("{options_mask} > 255 (manual §10-14)"),
            ));
        }
        let luno = match raw.fields().get(5) {
            Some(field) if field.is_empty() => None,
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("CPL LUNO no es ASCII"))?;
                if text.len() != 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::invalid_field(
                        "CPL LUNO",
                        format!("'{text}' debe ser 3 dígitos"),
                    ));
                }
                Some(text.to_owned())
            }
            None => None,
        };
        let mut timers = Vec::new();
        if let Some(field) = raw.fields().get(6) {
            let text = std::str::from_utf8(field)
                .map_err(|_| ParseError::malformed("CPL timers no es ASCII"))?;
            if !text.is_empty() {
                if text.len() % 5 != 0 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::malformed(
                        "CPL timers: pares timer(2)+ticks(3) concatenados (manual §10-14)",
                    ));
                }
                for chunk in text.as_bytes().chunks_exact(5) {
                    let timer: u8 = std::str::from_utf8(&chunk[..2])
                        .expect("ya validado ASCII")
                        .parse()
                        .map_err(|_| {
                            ParseError::invalid_field(
                                "CPL timer",
                                "no numérico o fuera de rango (00-99)",
                            )
                        })?;
                    let ticks: u8 = std::str::from_utf8(&chunk[2..])
                        .expect("ya validado ASCII")
                        .parse()
                        .map_err(|_| {
                            ParseError::invalid_field(
                                "CPL ticks",
                                "no numérico o fuera de rango (000-255)",
                            )
                        })?;
                    timers.push(TimerEntry { timer, ticks });
                }
            }
        }
        Ok(ConfigParams {
            options_mask,
            luno,
            timers,
        })
    }

    /// '5': cada campo = número(3) + entradas de 3 chars decimales (000–255).
    fn parse_fits(raw: &RawFields) -> Result<Vec<FitDef>, ParseError> {
        let mut fits = Vec::new();
        for field in Self::payload_fields(raw) {
            let text =
                std::str::from_utf8(field).map_err(|_| ParseError::malformed("FIT no es ASCII"))?;
            if text.len() < 3 {
                return Err(ParseError::malformed("FIT sin número"));
            }
            let (number_str, data) = text.split_at(3);
            let number: u16 = parse_digits(number_str, "FIT number")?;
            if number > 999 {
                return Err(ParseError::invalid_field(
                    "FIT number",
                    format!("{number} > 999 (manual §10-31)"),
                ));
            }
            if data.len() % 3 != 0 {
                return Err(ParseError::malformed(
                    "FIT data: entradas de 3 chars (manual §10-31)",
                ));
            }
            let mut entries = Vec::with_capacity(data.len() / 3);
            for group in data.as_bytes().chunks_exact(3) {
                let group_str = std::str::from_utf8(group)
                    .map_err(|_| ParseError::malformed("FIT entry no es ASCII"))?;
                let value: u16 = group_str.parse().map_err(|_| {
                    ParseError::invalid_field("FIT entry", format!("'{group_str}' no numérico"))
                })?;
                if value > 255 {
                    return Err(ParseError::invalid_field(
                        "FIT entry",
                        format!("'{group_str}' > 255 (rango 000-255, manual §10-31)"),
                    ));
                }
                entries.push(value as u8);
            }
            // §10-31: el terminal almacena 41 entradas; el exceso se ignora.
            entries.truncate(FIT_MAX_ENTRIES);
            fits.push(FitDef { number, entries });
        }
        Ok(fits)
    }

    /// '6': Configuration ID, 4 dígitos 0001–9999 (manual §10-33).
    fn parse_config_id(raw: &RawFields) -> Result<u16, ParseError> {
        let text = raw.ascii_field(4)?;
        if text.len() != 4 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError::invalid_field(
                "Config ID",
                format!("'{text}' debe ser 4 dígitos (manual §10-33)"),
            ));
        }
        let value: u16 = text
            .parse()
            .map_err(|_| ParseError::invalid_field("Config ID", "no numérico"))?;
        if !(1..=9999).contains(&value) {
            return Err(ParseError::invalid_field(
                "Config ID",
                format!("{value} fuera de 0001-9999 (manual §10-33)"),
            ));
        }
        Ok(value)
    }

    /// 'A': LUNO opcional (campo 'h') + pares opción/código (§10-21).
    fn parse_enhanced_config(raw: &RawFields) -> Result<Ecpl, ParseError> {
        let luno = match raw.fields().get(4) {
            Some(field) if field.is_empty() => None,
            Some(field) => {
                let text = std::str::from_utf8(field)
                    .map_err(|_| ParseError::malformed("ECPL LUNO no es ASCII"))?;
                if text.len() != 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::invalid_field(
                        "ECPL LUNO",
                        format!("'{text}' debe ser 3 dígitos (manual §10-21 campo 'h')"),
                    ));
                }
                Some(text.to_owned())
            }
            None => None,
        };
        let mut ecpl_options = Vec::new();
        if let Some(field) = raw.fields().get(5) {
            let text = std::str::from_utf8(field)
                .map_err(|_| ParseError::malformed("ECPL options no es ASCII"))?;
            if !text.is_empty() {
                if text.len() % 5 != 0 || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::malformed(
                        "ECPL options: pares opcion(2)+código(3) concatenados (manual §10-21)",
                    ));
                }
                for chunk in text.as_bytes().chunks_exact(5) {
                    let number: u8 = std::str::from_utf8(&chunk[..2])
                        .expect("ya validado ASCII")
                        .parse()
                        .map_err(|_| ParseError::invalid_field("ECPL option", "no numérico"))?;
                    let code: u16 = std::str::from_utf8(&chunk[2..])
                        .expect("ya validado ASCII")
                        .parse()
                        .map_err(|_| ParseError::invalid_field("ECPL code", "no numérico"))?;
                    ecpl_options.push(EcplOption { number, code });
                }
            }
        }
        Ok(Ecpl {
            luno,
            options: ecpl_options,
        })
    }

    /// 'C': decisión Q12 del SPEC — 10 chars del manual por defecto; 14 chars
    /// del driver real solo con `tolerate_driver_quirks`.
    fn parse_date_time(
        raw: &RawFields,
        options: &ParseOptions,
    ) -> Result<DateTimeLoad, ParseError> {
        let text = raw.ascii_field(4)?;
        if text.is_empty() {
            return Err(ParseError::invalid_field(
                "Date/Time",
                "campo obligatorio vacío (manual §10-41 exige YYMMDDHHMM)",
            ));
        }
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError::invalid_field(
                "Date/Time",
                format!("'{text}' debe ser solo dígitos"),
            ));
        }
        match text.len() {
            10 => {
                let yy: u16 = text[0..2]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "año no numérico"))?;
                // §10-41: 00-89 → 2000-2089; 90-99 → 1990-1999.
                let year = if yy <= 89 { 2000 + yy } else { 1900 + yy };
                let month: u8 = text[2..4]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "mes no numérico"))?;
                let day: u8 = text[4..6]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "día no numérico"))?;
                let hour: u8 = text[6..8]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "hora no numérica"))?;
                let minute: u8 = text[8..10]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "minuto no numérico"))?;
                validate_date(year, month, day, hour, minute)?;
                Ok(DateTimeLoad {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    quirk_14: false,
                })
            }
            14 if options.tolerant_driver_quirks => {
                let year: u16 = text[0..4]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "año no numérico"))?;
                let month: u8 = text[4..6]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "mes no numérico"))?;
                let day: u8 = text[6..8]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "día no numérico"))?;
                let hour: u8 = text[8..10]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "hora no numérica"))?;
                let minute: u8 = text[10..12]
                    .parse()
                    .map_err(|_| ParseError::invalid_field("Date/Time", "minuto no numérico"))?;
                // Los segundos (2 chars finales) se descartan: §10-41 los
                // pone en cero de todas formas.
                validate_date(year, month, day, hour, minute)?;
                Ok(DateTimeLoad {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    quirk_14: true,
                })
            }
            14 => Err(ParseError::invalid_field(
                "Date/Time",
                "14 chars (formato del driver actual; manual §10-41 = 10). \
                 Active tolerate_driver_quirks para aceptarlo (SPEC Q12)",
            )),
            other => Err(ParseError::invalid_field(
                "Date/Time",
                format!("{other} chars: se esperaban 10 (YYMMDDHHMM, manual §10-41)"),
            )),
        }
    }
}

/// Valida rangos de fecha/hora (manual §10-41: MM 01-12, DD 01-31,
/// HH 00-23, MM 00-59). Inválido → el motor responde reject (§10-87).
fn validate_date(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Result<(), ParseError> {
    if !(1..=12).contains(&month) {
        return Err(ParseError::invalid_field(
            "Date/Time",
            format!("mes {month} inválido"),
        ));
    }
    if !(1..=31).contains(&day) {
        return Err(ParseError::invalid_field(
            "Date/Time",
            format!("día {day} inválido"),
        ));
    }
    if hour > 23 {
        return Err(ParseError::invalid_field(
            "Date/Time",
            format!("hora {hour} inválida"),
        ));
    }
    if minute > 59 {
        return Err(ParseError::invalid_field(
            "Date/Time",
            format!("minuto {minute} inválido"),
        ));
    }
    let _ = year;
    Ok(())
}

/// Parsea un substring de dígitos con nombre de campo para el error.
fn parse_digits(text: &str, field: &'static str) -> Result<u16, ParseError> {
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::invalid_field(
            field,
            format!("'{text}' no numérico"),
        ));
    }
    text.parse()
        .map_err(|_| ParseError::invalid_field(field, format!("'{text}' fuera de rango")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScreenId;

    fn raw(body: &[u8]) -> RawFields {
        RawFields::parse(body)
    }

    fn strict() -> ParseOptions {
        ParseOptions::default()
    }

    fn tolerant() -> ParseOptions {
        ParseOptions {
            tolerant_driver_quirks: true,
        }
    }

    // ---- pantallas ('1') ----

    /// Estructura verificada contra el cable real: id (3 dígitos) pegado a
    /// los datos, un campo FS por pantalla (manual §10-11 h+i).
    #[test]
    fn screens_should_parse_concatenated_id_and_data() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1c000\x0c\x0fGDHOLA\x1c001\x0c\x0fGDBYE");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(screens.len(), 2);
        assert_eq!(
            screens[0].id,
            ScreenSelector::Customer(ScreenId::new(0).expect("en rango"))
        );
        assert_eq!(screens[0].data, b"\x0c\x0fGDHOLA");
        assert_eq!(
            screens[1].id,
            ScreenSelector::Customer(ScreenId::new(1).expect("en rango"))
        );
        assert_eq!(screens[0].keyboard, Vec::<Vec<u8>>::new());
    }

    /// Forma alternativa del campo 'h': 4 dígitos entre comillas (§10-11).
    #[test]
    fn screens_quoted_four_digit_id_should_parse() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1c\"0123\"HOLA");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(
            screens[0].id,
            ScreenSelector::Customer(ScreenId::new(123).expect("en rango"))
        );
        assert_eq!(screens[0].data, b"HOLA");
    }

    /// Pantallas de grupo: letra + 2 dígitos (C04) y letra + 4 (E0123, §10-11).
    #[test]
    fn screens_group_ids_should_parse() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1cC04texto\x1cE0123texto");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(
            screens[0].id,
            ScreenSelector::Group {
                letter: 'C',
                digits: 4
            }
        );
        assert_eq!(
            screens[1].id,
            ScreenSelector::Group {
                letter: 'E',
                digits: 123
            }
        );
    }

    /// 'l' mapea a 'u' (manual §10-11).
    #[test]
    fn screens_group_l_should_map_to_u() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1cl0123texto");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(
            screens[0].id,
            ScreenSelector::Group {
                letter: 'u',
                digits: 123
            }
        );
    }

    /// Teclado touch tras el GS (Tabla Nota 2 de §10-11), crudo en v1.
    #[test]
    fn screens_keyboard_data_after_gs_should_be_kept() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1c000HOLA\x1dtouch\x1djunk");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(screens[0].data, b"HOLA");
        assert_eq!(
            screens[0].keyboard,
            vec![b"touch".to_vec(), b"junk".to_vec()]
        );
    }

    /// El driver real cierra los mensajes con FS: queda un campo vacío final.
    #[test]
    fn screens_should_tolerate_trailing_empty_field() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1c000\x0c\x0f\x1c");
        let data = CustomisationData::parse(&raw, '1', &strict())
            .expect("mensaje válido")
            .expect("identificador '1'");
        let CustomisationData::Screens(screens) = data else {
            panic!("debería ser Screens");
        };
        assert_eq!(screens.len(), 1);
    }

    #[test]
    fn screens_short_field_should_be_invalid() {
        let raw = raw(b"3\x1c\x1c\x1c11\x1c10x5");
        assert!(CustomisationData::parse(&raw, '1', &strict()).is_err());
    }

    // ---- state tables ('2') ----

    #[test]
    fn state_tables_should_parse_number_type_and_entries() {
        // Estado sintético: tipo A + pantalla 050 + entradas de 3 chars.
        let raw = raw(b"3\x1c\x1c\x1c12\x1c000A050001051002002\x1c001K002000000000");
        let data = CustomisationData::parse(&raw, '2', &strict())
            .expect("mensaje válido")
            .expect("identificador '2'");
        let CustomisationData::StateTables(tables) = data else {
            panic!("debería ser StateTables");
        };
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].number.value(), 0);
        assert_eq!(tables[0].state_type, 'A');
        assert_eq!(tables[0].entries, "050001051002002");
        assert_eq!(tables[1].state_type, 'K');
    }

    #[test]
    fn state_tables_should_reject_number_255() {
        let raw = raw(b"3\x1c\x1c\x1c12\x1c255A050001051002002");
        assert!(CustomisationData::parse(&raw, '2', &strict()).is_err());
    }

    #[test]
    fn state_tables_should_reject_illegal_type() {
        let raw = raw(b"3\x1c\x1c\x1c12\x1c0009050001051002");
        assert!(CustomisationData::parse(&raw, '2', &strict()).is_err());
    }

    #[test]
    fn state_tables_should_reject_misaligned_entries() {
        let raw = raw(b"3\x1c\x1c\x1c12\x1c000A0500010510020");
        assert!(CustomisationData::parse(&raw, '2', &strict()).is_err());
    }

    // ---- CPL ('3') ----

    #[test]
    fn config_params_should_parse_options_luno_and_timers() {
        // Bloque fijo de 25 dígitos: h(1)+i(3)+j(3)+k(3)+l(3)+m(3)+n(9).
        // m (offset 13..16) = 009 → ready + buffer 12 dígitos (§10-14).
        let head = "0000000000000009000000000"; // 25 chars, m="009"
        let raw = raw(format!("3\x1c\x1c\x1c13\x1c{head}\x1c001\x1c03030").as_bytes());
        let data = CustomisationData::parse(&raw, '3', &strict())
            .expect("mensaje válido")
            .expect("identificador '3'");
        let CustomisationData::ConfigParams(params) = data else {
            panic!("debería ser ConfigParams");
        };
        assert_eq!(params.options_mask, 9);
        assert_eq!(params.luno.as_deref(), Some("001"));
        assert_eq!(
            params.timers,
            vec![TimerEntry {
                timer: 3,
                ticks: 30
            }]
        );
    }

    #[test]
    fn config_params_short_header_should_be_malformed() {
        let raw = raw(b"3\x1c\x1c\x1c13\x1c000");
        assert!(CustomisationData::parse(&raw, '3', &strict()).is_err());
    }

    // ---- FITs ('5') ----

    #[test]
    fn fits_should_decode_decimal_groups_to_bytes() {
        // FIT sintética: número 001 + PIDDX 000 + PFIID 255×5 (wildcard).
        let data_str = format!("001000{}", "255".repeat(5));
        let raw = raw(format!("3\x1c\x1c\x1c15\x1c{data_str}").as_bytes());
        let parsed = CustomisationData::parse(&raw, '5', &strict())
            .expect("mensaje válido")
            .expect("identificador '5'");
        let CustomisationData::Fits(fits) = parsed else {
            panic!("debería ser Fits");
        };
        assert_eq!(fits.len(), 1);
        assert_eq!(fits[0].number, 1);
        assert_eq!(&fits[0].entries[..6], &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    /// El driver real manda 40 entradas (SPEC Q11): debe aceptarse.
    #[test]
    fn fits_with_forty_entries_should_be_accepted() {
        let data_str = format!("000{}", "010".repeat(40));
        let raw = raw(format!("3\x1c\x1c\x1c15\x1c{data_str}").as_bytes());
        let parsed = CustomisationData::parse(&raw, '5', &strict())
            .expect("40 entradas son válidas (SPEC Q11)")
            .expect("identificador '5'");
        let CustomisationData::Fits(fits) = parsed else {
            panic!("debería ser Fits");
        };
        assert_eq!(fits[0].entries.len(), 40);
    }

    /// §10-31: más de 41 entradas → el exceso se ignora.
    #[test]
    fn fits_excess_entries_should_be_truncated_to_41() {
        let data_str = format!("000{}", "010".repeat(45));
        let raw = raw(format!("3\x1c\x1c\x1c15\x1c{data_str}").as_bytes());
        let parsed = CustomisationData::parse(&raw, '5', &strict())
            .expect("el exceso se ignora, no se rechaza (§10-31)")
            .expect("identificador '5'");
        let CustomisationData::Fits(fits) = parsed else {
            panic!("debería ser Fits");
        };
        assert_eq!(fits[0].entries.len(), 41);
    }

    #[test]
    fn fits_group_above_255_should_be_invalid() {
        let raw = raw(b"3\x1c\x1c\x1c15\x1c000256256256");
        assert!(CustomisationData::parse(&raw, '5', &strict()).is_err());
    }

    // ---- Config ID ('6') ----

    #[test]
    fn config_id_should_parse_four_digits() {
        let raw = raw(b"3\x1c\x1c\x1c16\x1c0001");
        let parsed = CustomisationData::parse(&raw, '6', &strict())
            .expect("mensaje válido")
            .expect("identificador '6'");
        assert_eq!(parsed, CustomisationData::ConfigId(1));
    }

    #[test]
    fn config_id_zero_should_be_invalid() {
        let raw = raw(b"3\x1c\x1c\x1c16\x1c0000");
        assert!(CustomisationData::parse(&raw, '6', &strict()).is_err());
    }

    // ---- ECPL ('A') ----

    #[test]
    fn ecpl_should_parse_luno_and_option_pairs() {
        // LUNO 002 + opciones sintéticas: 01/001 (ready) y 03/002 (fecha DDMM).
        let raw = raw(b"3\x1c\x1c\x1c1A\x1c002\x1c0100103002");
        let parsed = CustomisationData::parse(&raw, 'A', &strict())
            .expect("mensaje válido")
            .expect("identificador 'A'");
        let CustomisationData::EnhancedConfig(ecpl) = parsed else {
            panic!("debería ser EnhancedConfig");
        };
        assert_eq!(ecpl.luno.as_deref(), Some("002"));
        assert_eq!(
            ecpl.options,
            vec![
                EcplOption { number: 1, code: 1 },
                EcplOption { number: 3, code: 2 },
            ]
        );
    }

    /// El campo LUNO puede venir vacío (plantilla del driver real).
    #[test]
    fn ecpl_empty_luno_should_be_none() {
        let raw = raw(b"3\x1c\x1c\x1c1A\x1c\x1c01001");
        let parsed = CustomisationData::parse(&raw, 'A', &strict())
            .expect("mensaje válido")
            .expect("identificador 'A'");
        let CustomisationData::EnhancedConfig(ecpl) = parsed else {
            panic!("debería ser EnhancedConfig");
        };
        assert_eq!(ecpl.luno, None);
        assert_eq!(ecpl.options, vec![EcplOption { number: 1, code: 1 }]);
    }

    #[test]
    fn ecpl_misaligned_options_should_be_malformed() {
        let raw = raw(b"3\x1c\x1c\x1c1A\x1c002\x1c0100");
        assert!(CustomisationData::parse(&raw, 'A', &strict()).is_err());
    }

    // ---- Date/Time ('C'): decisión Q12 ----

    #[test]
    fn date_time_should_parse_manual_ten_char_format() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c2512312359");
        let parsed = CustomisationData::parse(&raw, 'C', &strict())
            .expect("YYMMDDHHMM conforme (manual §10-41)")
            .expect("identificador 'C'");
        assert_eq!(
            parsed,
            CustomisationData::DateTime(DateTimeLoad {
                year: 2025,
                month: 12,
                day: 31,
                hour: 23,
                minute: 59,
                quirk_14: false,
            })
        );
    }

    /// Regla de año del manual: 90-99 → 1990-1999 (§10-41).
    #[test]
    fn date_time_year_90s_should_map_to_19xx() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c9912312359");
        let parsed = CustomisationData::parse(&raw, 'C', &strict())
            .expect("válido")
            .expect("identificador 'C'");
        let CustomisationData::DateTime(dt) = parsed else {
            panic!("debería ser DateTime");
        };
        assert_eq!(dt.year, 1999);
    }

    /// La regresión actual del driver (14 chars) se rechaza en modo estricto.
    #[test]
    fn date_time_fourteen_chars_should_fail_when_strict() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c20251231235959");
        assert!(CustomisationData::parse(&raw, 'C', &strict()).is_err());
    }

    /// Con el flag tolerant (SPEC Q12) se acepta con marca de quirk.
    #[test]
    fn date_time_fourteen_chars_should_parse_with_tolerant_flag() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c20251231235959");
        let parsed = CustomisationData::parse(&raw, 'C', &tolerant())
            .expect("tolerante a la divergencia del driver")
            .expect("identificador 'C'");
        assert_eq!(
            parsed,
            CustomisationData::DateTime(DateTimeLoad {
                year: 2025,
                month: 12,
                day: 31,
                hour: 23,
                minute: 59,
                quirk_14: true,
            })
        );
    }

    #[test]
    fn date_time_empty_should_fail_even_when_tolerant() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c");
        assert!(CustomisationData::parse(&raw, 'C', &tolerant()).is_err());
    }

    #[test]
    fn date_time_invalid_month_should_fail() {
        let raw = raw(b"3\x1c\x1c\x1c1C\x1c2513312359");
        assert!(CustomisationData::parse(&raw, 'C', &strict()).is_err());
    }

    // ---- crudos (B/E/G) e identificadores desconocidos ----

    #[test]
    fn cassette_mapping_should_keep_raw_payload() {
        let raw = raw(b"3\x1c\x1c\x1c1E\x1cdatos\x1cmas");
        let parsed = CustomisationData::parse(&raw, 'E', &strict())
            .expect("passthrough crudo")
            .expect("identificador 'E'");
        assert_eq!(
            parsed,
            CustomisationData::CassetteMapping(vec![b"datos".to_vec(), b"mas".to_vec()])
        );
    }

    #[test]
    fn undocumented_identifier_should_return_none() {
        let raw = raw(b"3\x1c\x1c\x1c1Z\x1cdatos");
        assert_eq!(
            CustomisationData::parse(&raw, 'Z', &strict()).expect("estructura válida"),
            None
        );
    }
}
