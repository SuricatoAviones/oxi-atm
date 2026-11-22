//! Mensajes ATM→host y su serialización (SPEC §3; manual §9-2, §9-13).
//!
//! Layouts verificados contra dos fuentes independientes: las tablas del
//! manual y el builder de electron-atm que operó contra el driver real
//! (traza: el Solicited Status Ready sale como `22 <FS> 001 <FS> <FS> 9`).
//!
//! Convención de campos: clase+sub-clase en un solo campo ("11"/"22"), LUNO,
//! un campo de secuencia vacío, y luego el payload. Los campos opcionales del
//! TRQ ausentes viajan como campos vacíos (posiciones preservadas), patrón
//! del builder de electron-atm tolerado por el driver.

use crate::types::{Luno, Mcn};
use crate::wire::RawFields;
use crate::{ParseError, FS};

/// Descriptor de estado solicitado (manual §9-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDescriptor {
    /// '8': falla de dispositivo.
    DeviceFault,
    /// '9': listo (responde a comandos).
    Ready,
    /// 'A': rechazo de comando.
    CommandReject,
    /// 'B': variante de Ready para Transaction Reply (opción 1/001, §10-21).
    ReadyAlternate,
    /// 'C': rechazo específico (opción 12 activa, §10-88).
    SpecificCommandReject,
    /// 'F': estado del terminal (Terminal State, §9-19).
    TerminalState,
}

impl StatusDescriptor {
    /// Carácter que viaja en el cable.
    pub fn as_char(self) -> char {
        match self {
            Self::DeviceFault => '8',
            Self::Ready => '9',
            Self::CommandReject => 'A',
            Self::ReadyAlternate => 'B',
            Self::SpecificCommandReject => 'C',
            Self::TerminalState => 'F',
        }
    }
}

/// Transaction Request (manual §9-2, Tabla 9-1).
///
/// Los campos h–n (pistas, op-code, importe, buffers) son opcionales y los
/// elige la tabla del estado I (§2-25); los ausentes viajan vacíos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRequest {
    /// LUNO del terminal (3 dígitos).
    pub luno: Luno,
    /// Time Variant Number (8 chars 0-9/A-F). Solo con data security activa
    /// (Tabla 9-1 Nota 3); v1 lo deja en `None` (campo vacío).
    pub tvn: Option<String>,
    /// '0' = no imprimir al tope del recibo, '1' = imprimir (Tabla 9-1 f).
    pub top_of_receipt: char,
    /// Número de coordinación, rotativo (Tabla 9-1 g).
    pub mcn: Mcn,
    /// Pista 2 (≤ 39 chars, rango 0x30-0x3F).
    pub track2: Option<String>,
    /// Pista 3 (≤ 106 chars).
    pub track3: Option<String>,
    /// Buffer de código de operación (8 chars; estados D/E/Y).
    pub op_code: Option<String>,
    /// Importe: 8 o 12 dígitos, justificado a derecha con ceros (Tabla 9-1 k).
    pub amount: Option<String>,
    /// Buffer A: PIN encriptado (16 chars); sin claves cargadas no se envía
    /// (Tabla 9-1 Nota 1).
    pub buffer_a: Option<String>,
    /// Buffer B (mín. 3 chars si viene; ≤ 32).
    pub buffer_b: Option<String>,
    /// Buffer C (≤ 32).
    pub buffer_c: Option<String>,
    /// Pista 1 (≤ 78 chars). Viaja con su identificador '1' (Tabla 9-1 o/p).
    pub track1: Option<String>,
    /// Datos de estado de la transacción anterior (Tabla 9-1 q/r):
    /// serial(4) + último status(1) + notas dispensadas(20).
    pub status_data: Option<String>,
}

impl TransactionRequest {
    /// Serializa al cable. Valida los invariantes documentados para que un
    /// bug del motor no produzca un mensaje inválido silenciosamente.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        if !matches!(self.top_of_receipt, '0' | '1') {
            return Err(ParseError::invalid_field(
                "top of receipt",
                format!("'{}': debe ser '0' o '1' (§9-2)", self.top_of_receipt),
            ));
        }
        if let Some(amount) = &self.amount {
            let valid_len = amount.len() == 8 || amount.len() == 12;
            if !valid_len || !amount.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ParseError::invalid_field(
                    "amount",
                    format!("'{amount}': 8 o 12 dígitos (§9-2)"),
                ));
            }
        }
        if let Some(buffer_b) = &self.buffer_b {
            if buffer_b.len() < 3 {
                return Err(ParseError::invalid_field(
                    "buffer B",
                    format!("longitud {} < 3 (§9-2)", buffer_b.len()),
                ));
            }
        }

        let mut fields: Vec<Vec<u8>> = Vec::new();
        fields.push(b"11".to_vec());
        fields.push(self.luno.as_str().as_bytes().to_vec());
        // Campo de secuencia vacío (patrón verificado en la traza del host).
        fields.push(Vec::new());
        fields.push(self.tvn.clone().unwrap_or_default().into_bytes());
        fields.push(format!("{}{}", self.top_of_receipt, self.mcn.as_char()).into_bytes());
        for value in [
            &self.track2,
            &self.track3,
            &self.op_code,
            &self.amount,
            &self.buffer_a,
            &self.buffer_b,
            &self.buffer_c,
        ] {
            fields.push(value.clone().unwrap_or_default().into_bytes());
        }

        let raw = RawFields::new(fields)?;
        let mut bytes = raw.rebuild();

        // Grupo opcional pista 1: FS + identificador '1' + FS + datos
        // (Tabla 9-1 Nota 2: el grupo completo o nada).
        if let Some(track1) = &self.track1 {
            bytes.push(FS);
            bytes.push(b'1');
            bytes.push(FS);
            bytes.extend_from_slice(track1.as_bytes());
        }
        // Grupo opcional de estado: FS + identificador '2' + FS + datos.
        // (Tabla 9-1 Nota 5: el FS que precede a 'q' debe existir si hay
        // campos posteriores; sin campos posteriores no se agrega.)
        if let Some(status) = &self.status_data {
            bytes.push(FS);
            bytes.push(b'2');
            bytes.push(FS);
            bytes.extend_from_slice(status.as_bytes());
        }

        Ok(bytes)
    }
}

/// Mensaje del terminal hacia el host (SPEC §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalMessage {
    /// Clase 1/1: Transaction Request (§9-2). Boxeado: es la variante
    /// grande (12+ campos) y `large_enum_variant` exige equilibrio.
    TransactionRequest(Box<TransactionRequest>),
    /// Clase 2/2: Solicited Status (§9-13). Los `data` van tras el
    /// descriptor separados por FS; para `TerminalState` el primer dato es
    /// `g1` + primer subcampo concatenados (patrón del builder verificado
    /// contra el host real; g1 identifica el comando respondido, §9-19).
    SolicitedStatus {
        /// LUNO del terminal.
        luno: Luno,
        /// Descriptor del estado.
        descriptor: StatusDescriptor,
        /// Campos de datos ya formateados por el motor (puede ser vacío).
        data: Vec<String>,
    },
}

impl TerminalMessage {
    /// Serializa al cable (sin encuadre de transporte: eso es `oxi-net`).
    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        match self {
            Self::TransactionRequest(request) => request.to_bytes(),
            Self::SolicitedStatus {
                luno,
                descriptor,
                data,
            } => {
                let mut fields: Vec<Vec<u8>> = Vec::new();
                fields.push(b"22".to_vec());
                fields.push(luno.as_str().as_bytes().to_vec());
                fields.push(Vec::new());
                let mut first = descriptor.as_char().to_string().into_bytes();
                for value in data {
                    first.push(FS);
                    first.extend_from_slice(value.as_bytes());
                }
                fields.push(first);
                Ok(RawFields::new(fields)?.rebuild())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luno() -> Luno {
        Luno::new("001").expect("LUNO sintética válida")
    }

    /// Golden verificado contra la traza real del host: el Ready del
    /// electron-atm salió exactamente como estos bytes.
    #[test]
    fn ready_status_should_match_trace_bytes() {
        let msg = TerminalMessage::SolicitedStatus {
            luno: luno(),
            descriptor: StatusDescriptor::Ready,
            data: Vec::new(),
        };
        assert_eq!(
            msg.to_bytes().expect("serialización válida"),
            b"22\x1c001\x1c\x1c9"
        );
    }

    /// Terminal State con g1 + config id (patrón del builder empírico).
    #[test]
    fn terminal_state_status_should_append_data_fields() {
        let msg = TerminalMessage::SolicitedStatus {
            luno: luno(),
            descriptor: StatusDescriptor::TerminalState,
            data: vec!["60001".into()],
        };
        assert_eq!(
            msg.to_bytes().expect("serialización válida"),
            b"22\x1c001\x1c\x1cF\x1c60001"
        );
    }

    /// El TRQ serializado debe re-parsear campo a campo con RawFields
    /// (roundtrip estructural con nuestro propio splitter).
    #[test]
    fn request_minimal_should_roundtrip_fields() {
        let request = TransactionRequest {
            luno: luno(),
            tvn: None,
            top_of_receipt: '0',
            mcn: Mcn::new('\u{31}', false).expect("primer MCN del rango"),
            track2: None,
            track3: None,
            op_code: None,
            amount: None,
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: None,
            status_data: None,
        };
        let bytes = request.to_bytes().expect("TRQ válido");
        let parsed = RawFields::parse(&bytes);
        assert_eq!(parsed.fields()[0], b"11".to_vec());
        assert_eq!(parsed.fields()[1], b"001".to_vec());
        assert_eq!(parsed.fields()[2], Vec::<u8>::new());
        assert_eq!(parsed.fields()[3], Vec::<u8>::new());
        assert_eq!(parsed.fields()[4], b"01".to_vec());
        assert_eq!(parsed.fields().len(), 12);
    }

    /// Con pista 1 y estado anterior: identificadores '1'/'2' presentes
    /// (Tabla 9-1 o/p y q/r).
    #[test]
    fn request_with_track1_and_status_should_emit_identifier_groups() {
        let request = TransactionRequest {
            luno: luno(),
            tvn: None,
            top_of_receipt: '0',
            mcn: Mcn::new('\u{31}', false).expect("primer MCN del rango"),
            track2: Some(";1234=99?".into()),
            track3: None,
            op_code: Some("12345678".into()),
            amount: Some("00000100".into()),
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: Some("B4111...".into()),
            status_data: Some("00012".into()),
        };
        let bytes = request.to_bytes().expect("TRQ válido");
        let parsed = RawFields::parse(&bytes);
        let fields = parsed.fields();
        assert_eq!(fields[5], b";1234=99?".to_vec());
        assert_eq!(fields[7], b"12345678".to_vec());
        assert_eq!(fields[8], b"00000100".to_vec());
        // Grupo pista 1: identificador '1' + datos.
        assert_eq!(fields[12], b"1".to_vec());
        assert_eq!(fields[13], b"B4111...".to_vec());
        // Grupo estado: identificador '2' + datos.
        assert_eq!(fields[14], b"2".to_vec());
        assert_eq!(fields[15], b"00012".to_vec());
    }

    #[test]
    fn request_should_reject_bad_top_of_receipt() {
        let request = TransactionRequest {
            luno: luno(),
            tvn: None,
            top_of_receipt: 'X',
            mcn: Mcn::new('\u{31}', false).expect("primer MCN del rango"),
            track2: None,
            track3: None,
            op_code: None,
            amount: None,
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: None,
            status_data: None,
        };
        assert!(request.to_bytes().is_err());
    }

    /// §9-2: el importe es de 8 o 12 dígitos (opción 008 del ECPL).
    #[test]
    fn request_should_reject_amount_of_wrong_length() {
        let request = TransactionRequest {
            luno: luno(),
            tvn: None,
            top_of_receipt: '0',
            mcn: Mcn::new('\u{31}', false).expect("primer MCN del rango"),
            track2: None,
            track3: None,
            op_code: None,
            amount: Some("100".into()),
            buffer_a: None,
            buffer_b: None,
            buffer_c: None,
            track1: None,
            status_data: None,
        };
        assert!(request.to_bytes().is_err());
    }
}
