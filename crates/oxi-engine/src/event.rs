//! Eventos y acciones del motor (DESIGN §3: `handle(Event) -> Vec<Action>`).
//!
//! El motor no ejecuta efectos: devuelve intenciones y el runtime (oxi-ui)
//! las materializa (enviar, mostrar, dispensar, programar timers).

use std::time::Duration;

use oxi_core::{Fdk, FdkMask, NoteCounts, PrinterFlag, ScreenSelector, TerminalMessage};

/// Un evento que afecta al terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// El runtime arrancó el terminal (power-up).
    PoweredOn,
    /// Mensaje completo del host.
    HostMessage(oxi_core::HostMessage),
    /// Tecla del cardholder u operador.
    KeyPressed(Key),
    /// Tarjeta insertada con las pistas disponibles.
    CardInserted(Card),
    /// El cardholder retiró la tarjeta.
    CardTaken,
    /// El cardholder retiró el efectivo dispensado.
    CashTaken,
    /// Expiró un timer del manual (número 00-99, §10-17).
    TimerFired(u8),
}

/// Teclas que le importan al motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Tecla funcional lateral.
    Fdk(Fdk),
    /// Dígito del teclado numérico.
    Digit(char),
    /// Enter (equivale a FDK 'A' en importes, §2-17).
    Enter,
    /// Clear (borra la entrada).
    Clear,
    /// Cancel (aborta la transacción).
    Cancel,
}

/// Tarjeta sintética de prueba/perfil (pistas presentes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    /// Pista 1 (opcional).
    pub track1: Option<String>,
    /// Pista 2 (la habitual; contiene el FIID).
    pub track2: Option<String>,
    /// Pista 3 (opcional).
    pub track3: Option<String>,
}

/// Acciones que el motor pide al runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Enviar un mensaje al host.
    Send(TerminalMessage),
    /// Mostrar (o limpiar, con `None`) una pantalla.
    Display(Option<ScreenSelector>),
    /// Iniciar un timer del manual con su duración (ya resuelta de la config).
    StartTimer {
        /// Número de timer (00-99).
        timer: u8,
        /// Duración; `None` = 000 (infinito, no arrancar).
        after: Option<Duration>,
    },
    /// Cancelar un timer en curso.
    CancelTimer(u8),
    /// Devolver la tarjeta al cardholder.
    EjectCard,
    /// Dispensar billetes por casete.
    Dispense(NoteCounts),
    /// Imprimir un grupo (spool virtual en el runtime).
    Print {
        /// Destino de impresión.
        printer: PrinterFlag,
        /// Datos de impresión.
        data: String,
    },
    /// Traza de diagnóstico del motor (el runtime la mapea a `tracing`).
    Log(String),
    /// El terminal cambió de modo operativo.
    ModeChanged(oxi_core::OperationalMode),
}

/// Estado de espera del intérprete (qué evento desbloquea el estado actual).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// Esperando que el cardholder inserte tarjeta (estado A/T).
    Card,
    /// Esperando teclas con máscara de FDKs activas.
    Keys {
        /// Máscara de teclas (SPEC §5.3).
        mask: FdkMask,
    },
    /// Capturando dígitos hacia un buffer (F/R/H/X).
    Digits {
        /// Buffer destino.
        target: BufferId,
        /// Cantidad máxima de dígitos.
        max: usize,
        /// Eco local: `true` = una 'X' por tecla (H 000/002, §2-23);
        /// `false` = mostrar los datos (importe, H 001/003).
        masked: bool,
    },
    /// Capturando PIN (estado B).
    Pin {
        /// Máximo de dígitos (PMXPN de la FIT).
        max: usize,
    },
    /// Esperando el Transaction Reply del host (estado I).
    TransactionReply,
    /// Esperando que retiren el efectivo (timer 07).
    Cash,
}

/// Buffer destino de la captura de dígitos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferId {
    /// Buffer de importe.
    Amount,
    /// Buffer B.
    B,
    /// Buffer C.
    C,
}
