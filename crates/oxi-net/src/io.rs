//! Loops de lectura/escritura con encuadre y canales (SPEC §1; DESIGN §4).
//!
//! `FrameIo` es el par stream↔frames (testeable con `tokio::io::duplex`,
//! sin red). `pump` es el loop bidireccional con canales hacia/desde el
//! runtime, con dumps hex para diagnóstico (la utilidad que ya probabas valiosa
//! en el simulador C#).

use std::collections::VecDeque;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::codec::{FrameCodec, LenPrefix2Be};
use crate::error::NetError;

/// Par stream ↔ payloads enmarcados.
pub struct FrameIo<S> {
    stream: S,
    codec: LenPrefix2Be,
    read_buf: BytesMut,
    pending: VecDeque<Vec<u8>>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> FrameIo<S> {
    /// Envuelve un stream (duplex en tests, TcpStream en producción).
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            codec: LenPrefix2Be,
            read_buf: BytesMut::new(),
            pending: VecDeque::new(),
        }
    }

    /// Lee el próximo frame completo. `None` = EOF (el host cerró).
    ///
    /// Bloquea hasta que llegue un frame entero: TCP puede trocearlo como
    /// quiera; el codec acumula (tests de T2.1).
    pub async fn read_frame(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Ok(Some(frame));
            }
            let mut frames = Vec::new();
            self.codec.decode(&mut self.read_buf, &mut frames)?;
            self.pending.extend(frames);
            if self.pending.is_empty() {
                let mut chunk = [0u8; 4096];
                let read = self.stream.read(&mut chunk).await?;
                if read == 0 {
                    return Ok(None);
                }
                self.read_buf.extend_from_slice(&chunk[..read]);
            }
        }
    }

    /// Escribe un payload enmarcado completo.
    pub async fn write_frame(&mut self, payload: &[u8]) -> Result<(), NetError> {
        let mut out = BytesMut::new();
        self.codec.encode(payload, &mut out)?;
        self.stream.write_all(&out).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

/// Por qué terminó el loop de bombeo (para diagnóstico del runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpEnd {
    /// El stream llegó a EOF: el host cerró la conexión.
    StreamClosed,
    /// El canal del motor se cerró: no hay más payloads que enviar.
    EngineClosed,
}

/// Loop bidireccional: frames del host → `to_engine`; payloads del motor →
/// host. Termina cuando el stream hace EOF o el canal del motor se cierra.
///
/// Dump hex bidireccional con `tracing` (target `oxi_net`): habilitarlo con
/// `RUST_LOG=oxi_net=trace`.
pub async fn pump<S>(
    stream: S,
    to_engine: mpsc::Sender<Vec<u8>>,
    mut from_engine: mpsc::Receiver<Vec<u8>>,
) -> Result<PumpEnd, NetError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut io = FrameIo::new(stream);
    loop {
        tokio::select! {
            biased;

            frame = io.read_frame() => {
                let Some(frame) = frame? else {
                    tracing::info!(target: "oxi_net", "host cerró la conexión");
                    return Ok(PumpEnd::StreamClosed);
                };
                tracing::trace!(target: "oxi_net", "<< {} bytes: {}", frame.len(), hex_preview(&frame));
                if to_engine.send(frame).await.is_err() {
                    // El motor se fue: no tiene sentido seguir leyendo.
                    return Ok(PumpEnd::EngineClosed);
                }
            }

            payload = from_engine.recv() => {
                let Some(payload) = payload else {
                    tracing::info!(target: "oxi_net", "canal del motor cerrado");
                    return Ok(PumpEnd::EngineClosed);
                };
                tracing::trace!(target: "oxi_net", ">> {} bytes: {}", payload.len(), hex_preview(&payload));
                io.write_frame(&payload).await?;
            }
        }
    }
}

/// Vista previa hex de hasta 32 bytes (para no inundar el log con pantallas
/// completas de 2 KB).
fn hex_preview(bytes: &[u8]) -> String {
    const LIMIT: usize = 32;
    let shown = bytes.len().min(LIMIT);
    let mut text = String::with_capacity(shown * 3);
    for (index, byte) in bytes.iter().take(LIMIT).enumerate() {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(&format!("{byte:02X}"));
    }
    if bytes.len() > LIMIT {
        text.push_str(&format!(" (+{} más)", bytes.len() - LIMIT));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// Loopback sin red: dos mitades de duplex; el "host" escribe frames y
    /// el FrameIo los lee completos aunque lleguen troceados.
    #[tokio::test]
    async fn frame_io_should_reassemble_fragmented_frames() {
        let (client, mut host) = duplex(64);
        let mut io = FrameIo::new(client);

        // El "host" escribe el frame en tres pedazos.
        let payload = b"31\x1c\x1c\x1c31";
        let mut framed = BytesMut::new();
        LenPrefix2Be
            .encode(payload, &mut framed)
            .expect("encode válido");
        for chunk in framed.chunks(3) {
            host.write_all(chunk).await.expect("escritura del host");
            // Pequeño gap para que cada chunk llegue como lectura separada.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        let frame = io
            .read_frame()
            .await
            .expect("lectura válida")
            .expect("no hay EOF: el duplex sigue abierto");
        assert_eq!(frame, payload);
    }

    /// EOF del host → read_frame devuelve None (conexión cerrada).
    #[tokio::test]
    async fn frame_io_should_report_eof_as_none() {
        let (client, host) = duplex(64);
        drop(host); // el "host" cierra
        let mut io = FrameIo::new(client);
        let frame = io.read_frame().await.expect("lectura válida");
        assert!(frame.is_none());
    }

    /// Escritura → el "host" recibe el payload con prefijo BE.
    #[tokio::test]
    async fn frame_io_should_write_framed_payloads() {
        let (client, mut host) = duplex(64);
        let mut io = FrameIo::new(client);
        io.write_frame(b"HOLA").await.expect("escritura válida");

        // El frame mide 2 (prefijo) + 4 (HOLA) = 6 bytes exactos.
        let mut buf = [0u8; 6];
        host.read_exact(&mut buf).await.expect("lectura del host");
        assert_eq!(&buf[..2], &[0x00, 0x04]);
        assert_eq!(&buf[2..], b"HOLA");
    }

    /// Bombeo completo con canales: el host manda dos frames pegados, el
    /// motor recibe dos payloads; el motor contesta uno y el host lo lee.
    #[tokio::test]
    async fn pump_should_move_frames_in_both_directions() {
        let (client, mut host) = duplex(256);

        let (to_engine_tx, mut to_engine_rx) = mpsc::channel(8);
        let (from_engine_tx, from_engine_rx) = mpsc::channel(8);

        let pump_task = tokio::spawn(pump(client, to_engine_tx, from_engine_rx));

        // El "host" envía dos frames en una sola escritura.
        let mut wire = BytesMut::new();
        LenPrefix2Be.encode(b"UNO", &mut wire).expect("encode");
        LenPrefix2Be.encode(b"DOS", &mut wire).expect("encode");
        host.write_all(&wire).await.expect("escritura del host");

        // El motor recibe ambos y contesta.
        let first = to_engine_rx.recv().await.expect("frame UNO");
        assert_eq!(first, b"UNO");
        let second = to_engine_rx.recv().await.expect("frame DOS");
        assert_eq!(second, b"DOS");
        from_engine_tx
            .send(b"TRES".to_vec())
            .await
            .expect("canal vivo");

        // El host lee la respuesta del motor (prefijo + payload).
        let mut reply = [0u8; 6];
        host.read_exact(&mut reply).await.expect("lectura del host");
        assert_eq!(&reply[..2], &[0x00, 0x04]);
        assert_eq!(&reply[2..], b"TRES");

        // El host cierra: el pump termina con StreamClosed.
        drop(host);
        drop(from_engine_tx);
        let end = pump_task
            .await
            .expect("pump sin pánico")
            .expect("pump sin error");
        assert_eq!(end, PumpEnd::StreamClosed);
    }

    /// hex_preview: corta a 32 bytes y anota el resto.
    #[test]
    fn hex_preview_should_limit_to_thirty_two_bytes() {
        let bytes = vec![0xAB; 40];
        let text = hex_preview(&bytes);
        assert!(text.contains("+8 más"));
        let shown = text.split(' ').filter(|token| *token == "AB").count();
        assert_eq!(shown, 32, "solo los primeros 32 bytes van en hex");
    }
}
