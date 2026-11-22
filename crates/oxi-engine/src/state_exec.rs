//! Intérprete de state tables (SPEC §5; manual Cap. 2).
//!
//! Este módulo traduce una entrada de tabla al comportamiento del estado:
//! mostrar pantalla, esperar tarjeta/teclas, capturar dígitos, etc. El
//! intérprete completo por tipo llega en T3.4; este esqueleto cubre la
//! búsqueda, el default close y los hooks de eventos.

use oxi_core::StateNumber;

use crate::event::Action;
use crate::terminal::{default_close, Terminal};

/// Entra al estado `number` y ejecuta su acción.
pub(crate) fn enter_state(terminal: &mut Terminal, number: StateNumber) -> Vec<Action> {
    terminal.set_current(Some(number));
    let Some(state_type) = terminal.states().get(&number).map(|t| t.state_type) else {
        return default_close(terminal);
    };
    // El intérprete por tipo se completa en T3.4; por ahora, diagnóstico.
    vec![Action::Log(format!(
        "estado {} tipo '{state_type}' (intérprete por tipo: T3.4)",
        number
    ))]
}

/// Hook: tarjeta insertada mientras el estado espera tarjeta.
pub(crate) fn on_card_inserted(terminal: &mut Terminal) -> Vec<Action> {
    let _ = terminal;
    vec![Action::Log(
        "tarjeta insertada (intérprete del estado A: T3.4)".into(),
    )]
}

/// Hook: captura de dígitos aceptada con Enter.
pub(crate) fn on_digits_accepted(terminal: &mut Terminal) -> Vec<Action> {
    let _ = terminal;
    vec![Action::Log(
        "dígitos aceptados (transición del estado: T3.4)".into(),
    )]
}

/// Hook: timer del manual expirado.
pub(crate) fn on_timer(terminal: &mut Terminal, timer: u8) -> Vec<Action> {
    let _ = terminal;
    vec![Action::Log(format!(
        "timer {timer:02} expirado (Time-Out State: T3.7)"
    ))]
}

/// Helpers compartidos por los tests de la fase (customización sintética).
#[cfg(test)]
pub(crate) mod tests_support {
    use oxi_core::{
        CustomisationData, ScreenDef, ScreenId, ScreenSelector, StateNumber, StateTableDef,
    };

    use crate::terminal::{apply_customisation, Terminal};

    /// Carga una customización mínima y claramente sintética: estado 000 (A)
    /// que apunta al estado 002, estado 002 y un cierre 200.
    pub(crate) fn load_minimal_customisation(terminal: &mut Terminal) {
        let _ = apply_customisation(
            terminal,
            CustomisationData::StateTables(vec![
                StateTableDef {
                    number: StateNumber::new(0).expect("en rango"),
                    state_type: 'A',
                    // pantalla 010, rc1 001, good1 002, rc2 002, good2 002,
                    // rc3 002, card_return 000, no_fit 001
                    entries: "010001002002002002000001".into(),
                },
                StateTableDef {
                    number: StateNumber::new(2).expect("en rango"),
                    state_type: 'D',
                    entries: "010000000000000000000000".into(),
                },
                StateTableDef {
                    number: StateNumber::new(200).expect("en rango"),
                    state_type: 'J',
                    entries: "010000010010010010010010".into(),
                },
            ]),
        );
        let _ = apply_customisation(
            terminal,
            CustomisationData::Screens(vec![
                ScreenDef {
                    id: ScreenSelector::Customer(ScreenId::new(10).expect("en rango")),
                    data: b"\x0cBIENVENIDO".to_vec(),
                    keyboard: Vec::new(),
                },
                ScreenDef {
                    id: ScreenSelector::Customer(ScreenId::new(1).expect("en rango")),
                    data: b"\x0cERROR".to_vec(),
                    keyboard: Vec::new(),
                },
            ]),
        );
    }
}
