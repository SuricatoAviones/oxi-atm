//! Errores de parseo/serialización de mensajes NDC.
//!
//! Regla del crate (AGENTS.md): nada de pánico ante mensajes malformados; el
//! error viaja como valor y quien llama decide reject vs tolerancia (SPEC §4.5).

/// Error al interpretar un mensaje NDC o uno de sus campos.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Un campo con formato o valor inválido.
    #[error("campo '{field}' inválido: {reason}")]
    InvalidField {
        /// Nombre del campo (estático, para log sin alocar).
        field: &'static str,
        /// Detalle del problema detectado.
        reason: String,
    },

    /// El mensaje no tiene la estructura mínima esperada.
    #[error("mensaje malformado: {reason}")]
    Malformed {
        /// Detalle del problema de estructura.
        reason: String,
    },

    /// Un valor numérico fuera del rango documentado.
    #[error("valor '{value}' fuera de rango (esperado {expected})")]
    OutOfRange {
        /// Valor tal como vino en el cable.
        value: String,
        /// Rango esperado, en texto legible.
        expected: String,
    },
}

impl ParseError {
    /// Constructor breve para `InvalidField`.
    pub fn invalid_field(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidField {
            field,
            reason: reason.into(),
        }
    }

    /// Constructor breve para `Malformed`.
    pub fn malformed(reason: impl Into<String>) -> Self {
        Self::Malformed {
            reason: reason.into(),
        }
    }

    /// Código valor/calificador de Specific Command Reject (Tabla 10-37).
    ///
    /// Es un **subset**: cubre los errores que puede producir este parser.
    /// El manual define más combinaciones para hardware/CPM/RSA que el
    /// simulador todavía no modela. Si la opción 12 no está activa, el motor
    /// cae a Command Reject 'A' (no se envía este código).
    pub fn reject_code(&self) -> &'static str {
        match self {
            Self::InvalidField { field, reason } => match *field {
                "notes" => "B11",
                "MCN" => "B09",
                "FIT number" => "B10",
                "FIT entry" => "B07",
                "Override screen" => "B18",
                "function ID" => "B06",
                "command" => "B04",
                "modifier" => "B05",
                "print groups" => "A03",
                "next state" => "B08",
                "Date/Time" => "B08",
                "ITR active keys" => "B07",
                "ITR screen timer" | "ITR display flag" => "B08",
                _ if reason.contains("no numérico")
                    || reason.contains("no es ASCII")
                    || reason.contains("no es un número") =>
                {
                    "B07"
                }
                _ => "B08",
            },
            Self::OutOfRange { .. } => "B08",
            Self::Malformed { reason } => {
                if reason.contains("sub-clase") {
                    "B02"
                } else if reason.contains("clase") {
                    "B01"
                } else if reason.contains("separador") {
                    "A02"
                } else if reason.contains("impresión") || reason.contains("print") {
                    "A03"
                } else {
                    "A01"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los mensajes de error deben incluir el nombre del campo para el log.
    #[test]
    fn invalid_field_message_should_name_the_field() {
        let err = ParseError::invalid_field("LUNO", "vacío");
        assert!(err.to_string().contains("LUNO"));
    }

    #[test]
    fn reject_code_should_cover_common_fields() {
        assert_eq!(ParseError::invalid_field("MCN", "x").reject_code(), "B09");
        assert_eq!(ParseError::invalid_field("notes", "x").reject_code(), "B11");
        assert_eq!(
            ParseError::invalid_field("FIT number", "x").reject_code(),
            "B10"
        );
        assert_eq!(
            ParseError::invalid_field("Override screen", "x").reject_code(),
            "B18"
        );
        assert_eq!(
            ParseError::invalid_field("function ID", "x").reject_code(),
            "B06"
        );
        assert_eq!(
            ParseError::invalid_field("ITR active keys", "x").reject_code(),
            "B07"
        );
        assert_eq!(
            ParseError::invalid_field("ITR screen timer", "x").reject_code(),
            "B08"
        );
        assert_eq!(
            ParseError::invalid_field("LUNO", "no numérico").reject_code(),
            "B07"
        );
        assert_eq!(
            ParseError::OutOfRange {
                value: "999".into(),
                expected: "0-255".into(),
            }
            .reject_code(),
            "B08"
        );
        assert_eq!(
            ParseError::malformed("campo de clase vacío").reject_code(),
            "B01"
        );
        assert_eq!(
            ParseError::malformed("clase 3 sin sub-clase").reject_code(),
            "B02"
        );
        assert_eq!(
            ParseError::malformed("separador inesperado").reject_code(),
            "A02"
        );
        assert_eq!(
            ParseError::malformed("falta el campo 4").reject_code(),
            "A01"
        );
    }
}
