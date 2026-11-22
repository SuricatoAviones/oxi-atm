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
}
