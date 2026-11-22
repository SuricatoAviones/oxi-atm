//! Codec de encuadre del transporte (SPEC §1; DESIGN §4).
//!
//! El encuadre del host real del dueño está confirmado (nov-2026): prefijo de
//! longitud de 2 bytes big-endian seguido del mensaje ASCII (SPEC Q1). El
//! trait existe para tolerar variantes futuras sin rediseño (ADR D5); la
//! implementación por defecto es [`LenPrefix2BE`].

use bytes::{Buf, BufMut, BytesMut};

use crate::error::NetError;

/// Longitud del prefijo de longitud (2 bytes).
const PREFIX_LEN: usize = 2;

/// Tope del payload: el prefijo u16 ya lo limita a 65535 bytes.
const MAX_PAYLOAD: usize = u16::MAX as usize;

/// Un codec de encuadre: bytes del socket ↔ payloads completos de mensaje.
pub trait FrameCodec: Send {
    /// Extrae del buffer todos los frames completos disponibles y los agrega
    /// a `out`. Los bytes parciales quedan en `buf` hasta que llegue el
    /// resto (TCP no garantiza fronteras de mensaje).
    fn decode(&mut self, buf: &mut BytesMut, out: &mut Vec<Vec<u8>>) -> Result<(), NetError>;

    /// Escribe el payload enmarcado en `out`. Falla si el payload excede lo
    /// que el prefijo puede anunciar.
    fn encode(&mut self, payload: &[u8], out: &mut BytesMut) -> Result<(), NetError>;
}

/// Prefijo de longitud de 2 bytes big-endian + payload (confirmado por el
/// dueño contra su host; verificado además byte a byte en la traza de
/// electron-atm: `00 05 31 1c 1c 1c 31`).
#[derive(Debug, Clone, Copy, Default)]
pub struct LenPrefix2Be;

impl FrameCodec for LenPrefix2Be {
    fn decode(&mut self, buf: &mut BytesMut, out: &mut Vec<Vec<u8>>) -> Result<(), NetError> {
        loop {
            if buf.len() < PREFIX_LEN {
                // Ni siquiera llegó el prefijo completo.
                break;
            }
            let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
            if buf.len() < PREFIX_LEN + len {
                // Prefijo completo, payload incompleto: esperar más bytes.
                break;
            }
            buf.advance(PREFIX_LEN);
            // Nota: len == 0 produce un frame vacío; RawFields lo reportará
            // como mensaje malformado (un cuerpo NDC siempre tiene clase).
            out.push(buf.split_to(len).to_vec());
        }
        Ok(())
    }

    fn encode(&mut self, payload: &[u8], out: &mut BytesMut) -> Result<(), NetError> {
        let len = u16::try_from(payload.len()).map_err(|_| {
            NetError::Codec(format!(
                "payload de {} bytes excede el máximo de {MAX_PAYLOAD}",
                payload.len()
            ))
        })?;
        out.put_u16(len);
        out.extend_from_slice(payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn encode(payload: &[u8]) -> BytesMut {
        let mut codec = LenPrefix2Be;
        let mut out = BytesMut::new();
        codec.encode(payload, &mut out).expect("encode válido");
        out
    }

    fn decode_all(buf: &mut BytesMut) -> Vec<Vec<u8>> {
        let mut codec = LenPrefix2Be;
        let mut out = Vec::new();
        codec.decode(buf, &mut out).expect("decode siempre válido");
        out
    }

    /// Golden con los bytes reales de la traza del host: `00 05` + 5 bytes.
    #[test]
    fn single_frame_should_decode() {
        let mut buf = encode(b"31\x1c\x1c\x1c");
        assert_eq!(decode_all(&mut buf), vec![b"31\x1c\x1c\x1c".to_vec()]);
    }

    /// TCP puede entregar un frame en pedazos: el codec debe acumular sin
    /// emitir hasta completarlo (el bug clásico de encuadre que el simulador
    /// C# previo resolvía a mano).
    #[test]
    fn fragmented_frame_should_wait_for_completion() {
        let mut codec = LenPrefix2Be;
        let frame = encode(b"HOLA");
        let mut buf = BytesMut::new();

        // Solo el primer byte del prefijo.
        buf.extend_from_slice(&frame[..1]);
        let mut out = Vec::new();
        codec.decode(&mut buf, &mut out).expect("decode válido");
        assert!(out.is_empty(), "prefijo incompleto no emite nada");

        // Prefijo completo pero payload parcial.
        buf.extend_from_slice(&frame[1..4]);
        codec.decode(&mut buf, &mut out).expect("decode válido");
        assert!(out.is_empty(), "payload parcial no emite nada");

        // Resto del payload.
        buf.extend_from_slice(&frame[4..]);
        codec.decode(&mut buf, &mut out).expect("decode válido");
        assert_eq!(out, vec![b"HOLA".to_vec()]);
    }

    /// Varios mensajes pueden llegar pegoteados en un mismo segmento TCP.
    #[test]
    fn multiple_frames_in_one_segment_should_split() {
        let mut buf = encode(b"UNO");
        buf.extend_from_slice(&encode(b"DOS"));
        buf.extend_from_slice(&encode(b"TRES"));
        assert_eq!(
            decode_all(&mut buf),
            vec![b"UNO".to_vec(), b"DOS".to_vec(), b"TRES".to_vec()]
        );
    }

    /// Un prefijo anunciando más bytes de los presentes no es error: espera.
    #[test]
    fn announced_length_beyond_buffer_should_wait() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&10u16.to_be_bytes());
        buf.extend_from_slice(b"12345");
        assert!(decode_all(&mut buf).is_empty());
    }

    /// Frame vacío: el prefijo 0000 anuncia cero bytes; pasa como frame
    /// vacío y el parser superior lo rechaza como mensaje malformado.
    #[test]
    fn zero_length_frame_should_be_emitted_empty() {
        let mut buf = encode(b"");
        assert_eq!(decode_all(&mut buf), vec![Vec::<u8>::new()]);
    }

    /// El payload máximo del prefijo u16 debe pasar; uno más, no.
    #[test]
    fn encode_should_reject_payload_above_u16() {
        let mut codec = LenPrefix2Be;
        let big = vec![b'X'; MAX_PAYLOAD];
        let mut out = BytesMut::new();
        assert!(codec.encode(&big, &mut out).is_ok());

        let too_big = vec![b'X'; MAX_PAYLOAD + 1];
        assert!(codec.encode(&too_big, &mut out).is_err());
    }

    /// Propiedad: encode→decode es identidad para cualquier payload dentro
    /// del rango del prefijo.
    #[test]
    fn encode_decode_should_roundtrip() {
        proptest!(|(payload in proptest::collection::vec(any::<u8>(), 0..600))| {
            let mut buf = encode(&payload);
            assert_eq!(decode_all(&mut buf), vec![payload]);
        });
    }
}
