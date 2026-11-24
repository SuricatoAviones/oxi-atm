//! `oxi-core`: tipos y parser/serializador de mensajes NDC (APTRA Advance NDC).
//!
//! Este crate es la única fuente de verdad del formato del cable: mensajes como
//! enums, split por FS/GS y roundtrips verificados. No depende de tokio, I/O ni
//! interfaz (ver `docs/ARCHITECTURE.md` §2).

mod command;
mod customisation;
mod error;
mod host;
mod modes;
mod parse_terminal;
mod reply;
mod serialize;
mod terminal;
mod types;
mod wire;

pub use command::TerminalCommand;
pub use customisation::{
    ConfigParams, CustomisationData, DateTimeLoad, Ecpl, EcplOption, FitDef, ScreenDef,
    ScreenOverride, StateTableDef, TimerEntry,
};
pub use error::ParseError;
pub use host::{parse_host, HostMessage, ParseOptions};
pub use modes::{disposition, Disposition, ModeContext, OperationalMode};
pub use parse_terminal::parse_terminal;
pub use reply::{InteractiveTransactionResponse, TransactionReply};
pub use terminal::{StatusDescriptor, TerminalMessage, TransactionRequest};
pub use types::{
    CardReturnFlag, Fdk, FdkMask, Luno, Mcn, NoteCounts, PrintGroup, PrinterFlag, ScreenId,
    ScreenSelector, StateNumber, STATE_DISABLED,
};
pub use wire::{RawFields, FS, GS};

/// Nombre del crate, para verificación de armado del workspace.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// El nombre del crate debe ser `oxi-core` (test humo de scaffolding).
    #[test]
    fn crate_name_should_be_oxi_core() {
        assert_eq!(CRATE_NAME, "oxi-core");
    }
}
