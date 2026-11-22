//! Formación de PIN block ISO-0 (SPEC §12; manual §11-3…§11-6).
//!
//! v1: la formación del bloque es REAL (XOR del PIN field con el PAN block);
//! el cifrado DES/3DES llega en la fase 7. El FIT del driver real tiene
//! PINPD = 0x1F (bit 2 = "no cifrar"), así que el host espera el bloque sin
//! cifrar para verificación remota — consistente con esta fase.

/// Error de formación del PIN block.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// PIN de longitud inválida (4–12 dígitos).
    #[error("PIN inválido: {0}")]
    InvalidPin(String),
    /// El PAN no tiene dígitos suficientes.
    #[error("PAN inválido: {0}")]
    InvalidPan(String),
}

/// Forma el PIN block ISO-0 (16 hex, XOR de PIN field y PAN block).
///
/// - PIN field: `0` + largo (2 dígitos) + PIN + relleno `F` hasta 16.
/// - PAN block: 12 dígitos menos significativos del PAN (sin dígito
///   verificador) con relleno `0` a la izquierda hasta 16 (manual §11-3).
pub fn iso0_pin_block(pin: &str, pan: &str) -> Result<String, CryptoError> {
    if !(4..=12).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CryptoError::InvalidPin(format!(
            "se esperaban 4-12 dígitos, llegó '{pin}'"
        )));
    }
    let pan_field = pan.split('=').next().unwrap_or(pan);
    let pan_digits: String = pan_field.chars().filter(char::is_ascii_digit).collect();
    // El dígito verificador (último) no participa del bloque (§11-3).
    let pan_body = if pan_digits.len() > 12 {
        &pan_digits[pan_digits.len() - 13..pan_digits.len() - 1]
    } else if pan_digits.len() >= 12 {
        &pan_digits[pan_digits.len() - 12..]
    } else {
        return Err(CryptoError::InvalidPan(format!(
            "se esperaban al menos 12 dígitos, llegó '{pan_digits}'"
        )));
    };

    // ISO-0: control '0' + largo del PIN en UN nibble hex + PIN + relleno F
    // (manual §11-3/§11-4; PIN 1234 → campo "041234FFFFFFFFFF").
    let mut pin_field = format!("0{:X}{pin}", pin.len());
    while pin_field.len() < 16 {
        pin_field.push('F');
    }
    let pan_field = format!("{pan_body:0>16}");
    let block: String = pin_field
        .chars()
        .zip(pan_field.chars())
        .map(|(a, b)| {
            let x = a.to_digit(16).unwrap_or(0) ^ b.to_digit(16).unwrap_or(0);
            char::from_digit(x, 16).unwrap_or('0').to_ascii_uppercase()
        })
        .collect();
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vector sintético calculado a mano:
    /// PIN field: `041234FFFFFFFFFF`; PAN block (12 menos significativos
    /// de "5899411234567890" con relleno): `0000941123456789`; XOR →
    /// `0412A0EEDCBA9876`.
    #[test]
    fn iso0_block_should_xor_pin_and_pan_fields() {
        let block = iso0_pin_block("1234", "5899411234567890").expect("bloque válido");
        assert_eq!(block, "0412A0EEDCBA9876");
    }

    #[test]
    fn iso0_should_reject_short_pin() {
        assert!(iso0_pin_block("123", "5899411234567890").is_err());
    }

    #[test]
    fn iso0_should_reject_short_pan() {
        assert!(iso0_pin_block("1234", "12345").is_err());
    }

    /// El PAN puede venir con separadores (pista 2 completa).
    #[test]
    fn iso0_should_ignore_track2_separators() {
        let with_track = iso0_pin_block("1234", ";5899411234567890=2512").expect("válido");
        let clean = iso0_pin_block("1234", "5899411234567890").expect("válido");
        assert_eq!(with_track, clean);
    }
}
