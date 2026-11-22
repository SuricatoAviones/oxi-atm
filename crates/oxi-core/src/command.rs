//! Terminal Commands host→ATM, clase 1 (SPEC §4.1; manual §10-2…§10-6).
//!
//! El encabezado trae LUNO y número de secuencia opcionales que el terminal
//! ignora (manual §10-2), por lo que este tipo no los modela.

use crate::{wire::RawFields, ParseError};

/// Comando del host hacia el terminal (clase 1).
///
/// Los modificadores se preservan crudos: el **motor** (fase 3) valida los
/// valores documentados y aplica el rechazo que corresponde (manual §10-87).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCommand {
    /// '1': poner el terminal en servicio (manual §10-2).
    GoInService,
    /// '2': fuera de servicio. Modificador '0' = pantalla estándar,
    /// '1' = fuera de servicio temporal (manual §10-6).
    GoOutOfService {
        /// Modificador crudo ("" si no vino).
        modifier: String,
    },
    /// '3': pedir el Configuration ID (manual §10-2).
    SendConfigurationId,
    /// '4': pedir contadores de suministros (manual §10-2).
    SendSupplyCounters,
    /// '5': pedir tallies por dispositivo — **no soportado** en Advance NDC
    /// (manual §10-4); el motor responde con reject.
    SendTallyInformation {
        /// Modificador crudo (selector de dispositivo).
        modifier: String,
    },
    /// '6': pedir log de errores — **no soportado** (manual §10-2).
    SendErrorLog,
    /// '7': pedir información de configuración. Modificadores documentados
    /// (manual §10-5): 1 hardware, 2 supplies, 3 fitness, 4 tamper,
    /// 5 software release, 6 extendida, 7 config local, 8 notas BNA.
    SendConfigurationInformation {
        /// Modificador crudo.
        modifier: String,
    },
    /// '8': pedir fecha y hora del terminal (manual §10-2).
    SendDateAndTime,
    /// 'F': desconectar — solo dial-up VISA II (manual §10-4, Ap. O).
    Disconnect,
    /// 'G': no-op, mantener conexión — solo dial-up VISA II.
    NoOp,
}

impl TerminalCommand {
    /// Parsea el campo de comando (índice 3 del encabezado: código + resto).
    ///
    /// Devuelve `None` cuando el código no es ninguno de los documentados: el
    /// llamador lo envuelve en `HostMessage::Unknown` para que el motor decida
    /// el rechazo según el manual (§10-87: código ilegal).
    pub(crate) fn parse(raw: &RawFields) -> Result<Option<Self>, ParseError> {
        let field = raw.ascii_field(3)?;
        let mut chars = field.chars();
        let Some(code) = chars.next() else {
            return Err(ParseError::malformed("campo de comando vacío"));
        };
        let modifier: String = chars.collect();
        let command = match code {
            '1' => Self::GoInService,
            '2' => Self::GoOutOfService { modifier },
            '3' => Self::SendConfigurationId,
            '4' => Self::SendSupplyCounters,
            '5' => Self::SendTallyInformation { modifier },
            '6' => Self::SendErrorLog,
            '7' => Self::SendConfigurationInformation { modifier },
            '8' => Self::SendDateAndTime,
            'F' => Self::Disconnect,
            'G' => Self::NoOp,
            _ => return Ok(None),
        };
        Ok(Some(command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(body: &[u8]) -> RawFields {
        RawFields::parse(body)
    }

    #[test]
    fn in_service_should_parse_from_minimal_message() {
        let raw = raw(b"1\x1c\x1c\x1c1");
        let cmd = TerminalCommand::parse(&raw).expect("mensaje válido");
        assert_eq!(cmd, Some(TerminalCommand::GoInService));
    }

    #[test]
    fn out_of_service_should_keep_modifier() {
        let raw = raw(b"1\x1c\x1c\x1c21");
        let cmd = TerminalCommand::parse(&raw).expect("mensaje válido");
        assert_eq!(
            cmd,
            Some(TerminalCommand::GoOutOfService {
                modifier: "1".into()
            })
        );
    }

    #[test]
    fn config_info_should_keep_modifier() {
        let raw = raw(b"1\x1c\x1c\x1c77");
        let cmd = TerminalCommand::parse(&raw).expect("mensaje válido");
        assert_eq!(
            cmd,
            Some(TerminalCommand::SendConfigurationInformation {
                modifier: "7".into()
            })
        );
    }

    #[test]
    fn date_command_should_parse() {
        let raw = raw(b"1\x1c\x1c\x1c8");
        let cmd = TerminalCommand::parse(&raw).expect("mensaje válido");
        assert_eq!(cmd, Some(TerminalCommand::SendDateAndTime));
    }

    /// Código no documentado en el manual → None (el motor lo rechaza).
    #[test]
    fn undocumented_code_should_return_none() {
        let raw = raw(b"1\x1c\x1c\x1c9");
        assert_eq!(
            TerminalCommand::parse(&raw).expect("estructura válida"),
            None
        );
    }

    #[test]
    fn missing_command_field_should_be_malformed() {
        let raw = raw(b"1\x1c");
        assert!(TerminalCommand::parse(&raw).is_err());
    }

    #[test]
    fn empty_command_field_should_be_malformed() {
        let raw = raw(b"1\x1c\x1c\x1c");
        assert!(TerminalCommand::parse(&raw).is_err());
    }
}
