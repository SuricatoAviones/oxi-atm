//! Formación y cifrado del PIN block (SPEC §12; manual §11-3…§11-6, §11-18).
//!
//! - Formación ISO-0: XOR del PIN field con el PAN block (real).
//! - Cifrado: DES simple o 3DES EDE con claves de PRUEBA del perfil
//!   (`des` crate). El modo lo decide el perfil; el FIT del driver real
//!   (PINPD `1F`: dígito 1 = single, pad F) usa DES simple.
//! - KVV: cifrar 8 bytes cero y tomar 3 bytes (6 hex), §11-18.
//! - Claves del host: datos decimales, 3 chars por byte (§10-45).

use des::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use des::{Des, TdesEde2};

/// Error de formación o cifrado del PIN block.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// PIN de longitud inválida (4–12 dígitos).
    #[error("PIN inválido: {0}")]
    InvalidPin(String),
    /// El PAN no tiene dígitos suficientes.
    #[error("PAN inválido: {0}")]
    InvalidPan(String),
    /// Clave con longitud o codificación inválida.
    #[error("clave inválida: {0}")]
    InvalidKey(String),
    /// Bloque de 16 hex inválido.
    #[error("bloque inválido: {0}")]
    InvalidBlock(String),
}

/// Modo de cifrado de la clave de PIN (§11-6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyMode {
    /// DES simple (8 bytes; PINPD dígito 1 = single).
    #[default]
    Single,
    /// Triple DES EDE de doble longitud (16 bytes; §11-6 Nota 2).
    Triple,
}

/// Configuración de seguridad del simulador. Claves de PRUEBA, sintéticas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityConfig {
    /// Clave de trabajo del PIN: 16 hex (single) o 32 hex (triple).
    pub pin_key: Option<String>,
    /// Modo de la clave de PIN.
    pub pin_key_mode: KeyMode,
    /// Clave maestra ('A'): 16 hex.
    pub master_key: Option<String>,
    /// Clave local 'B' (entrada por supervisor): 16 hex.
    pub local_b_key: Option<String>,
}

impl SecurityConfig {
    /// Clave de trabajo decodificada (bytes), validando el modo.
    pub fn working_pin_key(&self) -> Option<Result<Vec<u8>, CryptoError>> {
        self.pin_key
            .as_deref()
            .map(|hex| decode_pin_key(hex, self.pin_key_mode))
    }

    /// Clave maestra decodificada.
    pub fn master_key_bytes(&self) -> Option<Result<Vec<u8>, CryptoError>> {
        self.master_key
            .as_deref()
            .map(|hex| decode_pin_key(hex, KeyMode::Single))
    }

    /// Clave local 'B' decodificada.
    pub fn local_b_key_bytes(&self) -> Option<Result<Vec<u8>, CryptoError>> {
        self.local_b_key
            .as_deref()
            .map(|hex| decode_pin_key(hex, KeyMode::Single))
    }
}

/// Decodifica una clave hex (16 hex = 8 bytes; 32 hex = 16 bytes).
fn decode_pin_key(hex: &str, mode: KeyMode) -> Result<Vec<u8>, CryptoError> {
    let bytes = hex_decode(hex)?;
    match (mode, bytes.len()) {
        (KeyMode::Single, 8) | (KeyMode::Triple, 16) => Ok(bytes),
        (KeyMode::Single, other) => Err(CryptoError::InvalidKey(format!(
            "single: se esperaban 8 bytes, llegaron {other}"
        ))),
        (KeyMode::Triple, other) => Err(CryptoError::InvalidKey(format!(
            "triple: se esperaban 16 bytes, llegaron {other}"
        ))),
    }
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, CryptoError> {
    if hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CryptoError::InvalidKey(format!("'{hex}' no es hex")));
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .map_err(|_| CryptoError::InvalidKey(format!("'{hex}' no es hex")))
        })
        .collect()
}

/// Cifra un bloque de 8 bytes (DES simple o 3DES EDE según el modo).
pub fn encrypt_block(key: &[u8], block: [u8; 8], mode: KeyMode) -> Result<[u8; 8], CryptoError> {
    let mut data = block;
    match mode {
        KeyMode::Single => {
            let cipher = Des::new_from_slice(key)
                .map_err(|_| CryptoError::InvalidKey("DES necesita 8 bytes".into()))?;
            cipher.encrypt_block((&mut data).into());
        }
        KeyMode::Triple => {
            let cipher = TdesEde2::new_from_slice(key)
                .map_err(|_| CryptoError::InvalidKey("3DES necesita 16 bytes".into()))?;
            cipher.encrypt_block((&mut data).into());
        }
    }
    Ok(data)
}

/// Descifra un bloque de 8 bytes (para verificación y cambios de clave).
pub fn decrypt_block(key: &[u8], block: [u8; 8], mode: KeyMode) -> Result<[u8; 8], CryptoError> {
    let mut data = block;
    match mode {
        KeyMode::Single => {
            let cipher = Des::new_from_slice(key)
                .map_err(|_| CryptoError::InvalidKey("DES necesita 8 bytes".into()))?;
            cipher.decrypt_block((&mut data).into());
        }
        KeyMode::Triple => {
            let cipher = TdesEde2::new_from_slice(key)
                .map_err(|_| CryptoError::InvalidKey("3DES necesita 16 bytes".into()))?;
            cipher.decrypt_block((&mut data).into());
        }
    }
    Ok(data)
}

/// Cifra el bloque ISO-0 (16 hex) y devuelve el cifrado en 16 hex.
pub fn encrypt_pin_block(
    block_hex: &str,
    key: &[u8],
    mode: KeyMode,
) -> Result<String, CryptoError> {
    let block = block_bytes(block_hex)?;
    let encrypted = encrypt_block(key, block, mode)?;
    Ok(hex_encode(&encrypted))
}

/// Descifra un bloque de PIN y devuelve el ISO-0 en hex (verificación).
pub fn decrypt_pin_block(
    block_hex: &str,
    key: &[u8],
    mode: KeyMode,
) -> Result<String, CryptoError> {
    let block = block_bytes(block_hex)?;
    let decrypted = decrypt_block(key, block, mode)?;
    Ok(hex_encode(&decrypted))
}

fn block_bytes(block_hex: &str) -> Result<[u8; 8], CryptoError> {
    let bytes = hex_decode(block_hex)?;
    bytes
        .try_into()
        .map_err(|_| CryptoError::InvalidBlock(format!("'{block_hex}' no son 8 bytes")))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// KVV: cifrar 8 bytes cero con la clave y tomar los 3 primeros bytes en
/// hex (6 chars, §11-18).
pub fn kvv(key: &[u8], mode: KeyMode) -> Result<String, CryptoError> {
    let encrypted = encrypt_block(key, [0u8; 8], mode)?;
    Ok(hex_encode(&encrypted[..3]))
}

/// Decodifica una clave del host en formato decimal (3 chars por byte,
/// §10-45) a bytes.
pub fn decode_decimal_key(text: &str) -> Result<Vec<u8>, CryptoError> {
    if text.is_empty() || text.len() % 3 != 0 {
        return Err(CryptoError::InvalidKey(format!(
            "longitud decimal {} no es múltiplo de 3",
            text.len()
        )));
    }
    text.as_bytes()
        .chunks_exact(3)
        .map(|chunk| {
            let value: u16 = std::str::from_utf8(chunk)
                .map_err(|_| CryptoError::InvalidKey("clave no ASCII".into()))?
                .parse()
                .map_err(|_| CryptoError::InvalidKey("clave no decimal".into()))?;
            u8::try_from(value).map_err(|_| CryptoError::InvalidKey(format!("byte {value} > 255")))
        })
        .collect()
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
    /// de "9999991234567890" con relleno): `0000999123456789`; XOR →
    /// `0412AD6EDCBA9876`.
    #[test]
    fn iso0_block_should_xor_pin_and_pan_fields() {
        let block = iso0_pin_block("1234", "9999991234567890").expect("bloque válido");
        assert_eq!(block, "0412AD6EDCBA9876");
    }

    #[test]
    fn iso0_should_reject_short_pin() {
        assert!(iso0_pin_block("123", "9999991234567890").is_err());
    }

    #[test]
    fn iso0_should_reject_short_pan() {
        assert!(iso0_pin_block("1234", "12345").is_err());
    }

    /// El PAN puede venir con separadores (pista 2 completa).
    #[test]
    fn iso0_should_ignore_track2_separators() {
        let with_track = iso0_pin_block("1234", ";9999991234567890=2512").expect("válido");
        let clean = iso0_pin_block("1234", "9999991234567890").expect("válido");
        assert_eq!(with_track, clean);
    }

    /// Vector DES publicado (FIPS): clave 133457799BBCDFF1, bloque
    /// 0123456789ABCDEF → 85E813540F0AB405.
    #[test]
    fn single_des_should_match_known_answer() {
        let key = hex_decode("133457799BBCDFF1").expect("clave hex");
        let block = block_bytes("0123456789ABCDEF").expect("bloque");
        let encrypted = encrypt_block(&key, block, KeyMode::Single).expect("cifra");
        assert_eq!(hex_encode(&encrypted), "85E813540F0AB405");
    }

    /// Cifrado y descifrado del PIN block: el resultado del decrypt es el
    /// ISO-0 original (lo que verificará el host con su HSM).
    #[test]
    fn pin_block_should_roundtrip_through_des() {
        let iso0 = iso0_pin_block("1234", "9999991234567890").expect("iso0");
        let key = hex_decode("0F1E2D3C4B5A6978").expect("clave");
        let encrypted = encrypt_pin_block(&iso0, &key, KeyMode::Single).expect("cifra");
        assert_eq!(encrypted.len(), 16);
        assert_ne!(encrypted, iso0, "el bloque cifrado difiere del ISO-0");
        let decrypted = decrypt_pin_block(&encrypted, &key, KeyMode::Single).expect("descifra");
        assert_eq!(decrypted, iso0);
    }

    /// 3DES EDE de doble longitud: roundtrip con clave de 16 bytes.
    #[test]
    fn triple_des_should_roundtrip() {
        let iso0 = iso0_pin_block("9876", "9999990000000000").expect("iso0");
        let key = hex_decode("0F1E2D3C4B5A69788796A5B4C3D2E1F0").expect("clave");
        let encrypted = encrypt_pin_block(&iso0, &key, KeyMode::Triple).expect("cifra");
        let decrypted = decrypt_pin_block(&encrypted, &key, KeyMode::Triple).expect("descifra");
        assert_eq!(decrypted, iso0);
    }

    /// KVV determinista: 6 hex, mencionado en §11-18.
    #[test]
    fn kvv_should_be_six_hex_chars() {
        let key = hex_decode("0123456789ABCDEF").expect("clave");
        let value = kvv(&key, KeyMode::Single).expect("kvv");
        assert_eq!(value.len(), 6);
        assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
        // Desencriptar el bloque de ceros no depende del estado: repetible.
        assert_eq!(value, kvv(&key, KeyMode::Single).expect("kvv"));
    }

    /// Claves del host: decimal de 3 chars por byte → bytes (§10-45).
    #[test]
    fn decimal_key_should_decode_to_bytes() {
        // 000 255 032 → 0x00 0xFF 0x20
        let bytes = decode_decimal_key("000255032").expect("clave decimal");
        assert_eq!(bytes, vec![0x00, 0xFF, 0x20]);
    }

    #[test]
    fn decimal_key_with_bad_chunk_should_fail() {
        assert!(decode_decimal_key("000255999").is_err());
        assert!(decode_decimal_key("00").is_err());
    }

    /// La configuración valida el largo según el modo.
    #[test]
    fn security_config_should_validate_key_length_by_mode() {
        let single = SecurityConfig {
            pin_key: Some("0F1E2D3C4B5A6978".into()),
            ..SecurityConfig::default()
        };
        assert!(single.working_pin_key().expect("hay clave").is_ok());

        let wrong = SecurityConfig {
            pin_key: Some("0F1E2D3C4B5A69788796A5B4C3D2E1F0".into()),
            pin_key_mode: KeyMode::Single,
            ..SecurityConfig::default()
        };
        assert!(wrong.working_pin_key().expect("hay clave").is_err());
    }
}
