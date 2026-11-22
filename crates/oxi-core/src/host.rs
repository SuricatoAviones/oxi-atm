//! Dispatcher de mensajes host→ATM (SPEC §2, §4).
//!
//! `parse_host` clasifica por clase y delega en el parser tipado de cada una.
//! Lo que no está modelado todavía (o no existe en el manual) vuelve como
//! [`HostMessage::Unknown`] con el mensaje crudo preservado: el motor decide
//! el rechazo según §10-87/§10-95, el parser no inventa política.

use crate::command::TerminalCommand;
use crate::wire::RawFields;
use crate::ParseError;

/// Opciones de parseo que provienen del perfil del simulador (no del cable).
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    /// Acepta las divergencias conocidas del driver real del dueño con
    /// warning (SPEC §14 Q12): hoy, el Date/Time de 14 chars
    /// `yyyyMMddHHmmss` en lugar de los 10 del manual §10-41.
    pub tolerant_driver_quirks: bool,
}

/// Mensaje completo host→ATM, ya clasificado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMessage {
    /// Clase 1: comandos del terminal.
    TerminalCommand(TerminalCommand),
    /// Clase no modelada o código/identificador no reconocido. El mensaje
    /// crudo queda preservado para diagnóstico y para la política de rechazo
    /// del motor (manual §10-87, §10-95).
    Unknown {
        /// Mensaje tal como llegó (incluye la clase).
        raw: RawFields,
    },
}

/// Clasifica un mensaje crudo del host.
///
/// El campo de clase puede traer un flag de respuesta ignorable pegado
/// (manual §10-2: "included for future use and ignored by the terminal").
pub fn parse_host(raw: &RawFields, _options: &ParseOptions) -> Result<HostMessage, ParseError> {
    let head = raw.ascii_field(0)?;
    let Some(class) = head.chars().next() else {
        return Err(ParseError::malformed("campo de clase vacío"));
    };
    match class {
        '1' => match TerminalCommand::parse(raw)? {
            Some(command) => Ok(HostMessage::TerminalCommand(command)),
            None => Ok(HostMessage::Unknown { raw: raw.clone() }),
        },
        // Clases 3 (customización/TR interactivo/claves), 4 (Transaction
        // Reply), 6 (EJ) y 8 (EMV) se incorporan en T1.4–T1.6.
        _ => Ok(HostMessage::Unknown { raw: raw.clone() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_one_should_dispatch_to_terminal_command() {
        let raw = RawFields::parse(b"1\x1c\x1c\x1c1");
        let msg = parse_host(&raw, &ParseOptions::default()).expect("mensaje válido");
        assert_eq!(
            msg,
            HostMessage::TerminalCommand(TerminalCommand::GoInService)
        );
    }

    #[test]
    fn unknown_class_should_be_preserved() {
        let raw = RawFields::parse(b"4\x1c\x1c\x1c080");
        let msg = parse_host(&raw, &ParseOptions::default()).expect("mensaje válido");
        assert_eq!(msg, HostMessage::Unknown { raw });
    }

    #[test]
    fn undocumented_command_should_be_unknown_not_error() {
        let raw = RawFields::parse(b"1\x1c\x1c\x1c9");
        let msg = parse_host(&raw, &ParseOptions::default()).expect("estructura válida");
        assert!(matches!(msg, HostMessage::Unknown { .. }));
    }

    #[test]
    fn empty_body_should_be_malformed() {
        let raw = RawFields::parse(b"");
        assert!(parse_host(&raw, &ParseOptions::default()).is_err());
    }
}
