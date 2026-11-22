//! Errores de transporte (oxi-net).

/// Error de red/encuadre de `oxi-net`.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// Error de I/O subyacente (TCP cerrado, timeouts, etc.).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// El codec no puede procesar los bytes recibidos o el payload a enviar.
    #[error("codec: {0}")]
    Codec(String),

    /// Un canal interno se cerró (el extremo que alimenta el loop se fue).
    #[error("canal cerrado")]
    ChannelClosed,
}
