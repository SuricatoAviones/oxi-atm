//! Encuadre del transporte del host falso: prefijo de 2 bytes big-endian
//! (el mismo que usa `oxi-net`; SPEC §1 lo fija como contrato del host).
//!
//! Se mantiene deliberadamente pequeño y sin depender de `oxi-net`
//! (ARCHITECTURE §2: el mock no depende del transporte del ATM), pero el
//! formato debe permanecer idéntico: 2 bytes BE + payload ASCII.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::host::HostError;

/// Lee un frame completo (prefijo BE de 2 bytes + payload). `None` = EOF.
pub async fn read_frame<S>(stream: &mut S) -> Result<Option<Vec<u8>>, HostError>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = [0u8; 2];
    match stream.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(HostError::Io(err)),
    }
    let len = usize::from(u16::from_be_bytes(prefix));
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

/// Escribe un frame (prefijo BE de 2 bytes + payload).
pub async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), HostError>
where
    S: AsyncWrite + Unpin,
{
    let len = u16::try_from(payload.len())
        .map_err(|_| HostError::Framing("payload excede u16".into()))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}
