//! Terminal NDC: stores de customización, buffers y loop de eventos
//! (SPEC §5; DESIGN §3).
//!
//! El motor es puro y determinista: entra un `Event`, salen `Action`s. No
//! conoce tokio, la red ni la interfaz. Los timers se piden con su número del
//! manual y la duración ya resuelta desde la configuración.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use oxi_core::{
    ConfigParams, CustomisationData, DateTimeLoad, Ecpl, FitDef, Luno, Mcn, OperationalMode,
    ScreenDef, ScreenSelector, StateNumber, StateTableDef, StatusDescriptor, TerminalMessage,
};

use crate::event::{Action, BufferId, Card, Event, Key, Pending};

/// Duración por defecto de un timer no configurado (manual §10-14 Nota 7:
/// 30 ticks de 800 ms ≈ 24 s; aquí se usa 30 s, el default del Cap. 7).
const DEFAULT_TIMER: Duration = Duration::from_secs(30);

/// Estado numero que el terminal usa como cierre por defecto ante un next
/// state inválido (SPEC §14 Q10: el manual no lo define; el driver real usa
/// la batería 200+ para cierres; configurable en el futuro).
const DEFAULT_CLOSE_STATE: u16 = 200;

/// Buffers de la transacción (manual §2-4: qué inicializa el estado A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffers {
    /// Dígitos de PIN capturados (en claro solo en memoria del simulador;
    /// en el TRQ viaja encriptado, fase 7).
    pub pin: String,
    /// Buffer de propósito general B.
    pub b: String,
    /// Buffer de propósito general C.
    pub c: String,
    /// Importe, rellenado a ceros (8 o 12 dígitos según opción 1/008).
    pub amount: String,
    /// Buffer de código de operación (8 posiciones, rellenas con espacio).
    pub opcode: [char; 8],
    /// Keycode de la última FDK (lo usa el estado W).
    pub fdk: Option<char>,
}

impl Default for Buffers {
    fn default() -> Self {
        Self {
            pin: String::new(),
            b: String::new(),
            c: String::new(),
            amount: "00000000".into(),
            opcode: [' '; 8],
            fdk: None,
        }
    }
}

impl Buffers {
    /// Inicialización al entrar al estado A (manual §2-4): tarjeta y buffers
    /// de propósito general vacíos, importe a ceros, op-code a espacios.
    pub(crate) fn reset_for_transaction(&mut self) {
        self.pin.clear();
        self.b.clear();
        self.c.clear();
        self.amount = "00000000".into();
        self.opcode = [' '; 8];
        self.fdk = None;
    }
}

/// Configuración del terminal que llega del host (CPL/ECPL, SPEC §11).
#[derive(Debug, Clone)]
pub(crate) struct Config {
    luno: Luno,
    /// Opciones del ECPL: número → código (se preservan todas; las no
    /// documentadas se ignoran con warning, SPEC Q13).
    options: HashMap<u8, u16>,
    /// Timers del CPL en ticks de 800 ms.
    timers_cpl: HashMap<u8, u8>,
    /// Timers del ECPL en segundos.
    timers_ecpl: HashMap<u8, u8>,
}

impl Config {
    fn new() -> Self {
        Self {
            luno: Luno::new("000").expect("LUNO por defecto válido"),
            options: HashMap::new(),
            timers_cpl: HashMap::new(),
            timers_ecpl: HashMap::new(),
        }
    }

    /// Duración de un timer: ECPL (segundos) > CPL (ticks × 800 ms) >
    /// default. `None` = 000: el manual lo toma como infinito.
    pub(crate) fn timer_duration(&self, timer: u8) -> Option<Duration> {
        if let Some(seconds) = self.timers_ecpl.get(&timer) {
            return (*seconds > 0).then(|| Duration::from_secs((*seconds).into()));
        }
        if let Some(ticks) = self.timers_cpl.get(&timer) {
            return (*ticks > 0).then(|| Duration::from_millis(u64::from(*ticks) * 800));
        }
        Some(DEFAULT_TIMER)
    }

    /// ¿Está activa la opción ECPL con este código? (helper de la matriz).
    fn option(&self, number: u8) -> Option<u16> {
        self.options.get(&number).copied()
    }
}

/// El terminal NDC.
#[derive(Debug)]
pub struct Terminal {
    // --- customización descargada (los loads la reemplazan) ---
    states: HashMap<StateNumber, StateTableDef>,
    screens: HashMap<ScreenSelector, ScreenDef>,
    fits: BTreeMap<u16, FitDef>,
    config: Config,
    config_id: Option<u16>,
    clock: Option<DateTimeLoad>,

    // --- estado corriente ---
    mode: OperationalMode,
    current: Option<StateNumber>,
    buffers: Buffers,
    card: Option<Card>,
    mcn: Mcn,
    pending: Option<Pending>,
    /// ¿Hay una transacción esperando el Transaction Reply? (Tabla 10-39).
    awaiting_reply: bool,
    /// Pantalla mostrada actualmente (para el runtime/UI).
    screen: Option<ScreenSelector>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    /// Terminal recién energizado, sin customización.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            screens: HashMap::new(),
            fits: BTreeMap::new(),
            config: Config::new(),
            config_id: None,
            clock: None,
            mode: OperationalMode::PowerUp,
            current: None,
            buffers: Buffers::default(),
            card: None,
            mcn: Mcn::new(Mcn::FIRST, false).expect("primer MCN válido"),
            pending: None,
            awaiting_reply: false,
            screen: None,
        }
    }

    // --- consultas (para el runtime/UI y los tests) ---

    /// Modo operativo actual.
    pub fn mode(&self) -> OperationalMode {
        self.mode
    }

    /// Estado actual (si hay uno en ejecución).
    pub fn current_state(&self) -> Option<StateNumber> {
        self.current
    }

    /// Pantalla actual.
    pub fn screen(&self) -> Option<ScreenSelector> {
        self.screen
    }

    /// Buffers (lectura para la UI).
    pub fn buffers(&self) -> &Buffers {
        &self.buffers
    }

    /// Config ID reportado por el host (None = 0000 de instalación).
    pub fn config_id(&self) -> Option<u16> {
        self.config_id
    }

    /// LUNO configurado por el host.
    pub fn luno(&self) -> &Luno {
        &self.config.luno
    }

    /// Cantidad de estados descargados (diagnóstico).
    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    // --- loop de eventos ---

    /// Procesa un evento y devuelve las acciones a ejecutar.
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::PoweredOn => self.handle_powered_on(),
            Event::HostMessage(message) => crate::host_ops::handle_host(self, message),
            Event::KeyPressed(key) => self.handle_key(key),
            Event::CardInserted(card) => self.handle_card_inserted(card),
            Event::CardTaken => {
                self.card = None;
                vec![Action::Log("tarjeta retirada por el cardholder".into())]
            }
            Event::CashTaken => {
                if self.pending == Some(Pending::Cash) {
                    self.pending = None;
                    vec![Action::CancelTimer(7)]
                } else {
                    Vec::new()
                }
            }
            Event::TimerFired(timer) => self.handle_timer(timer),
        }
    }

    fn handle_powered_on(&mut self) -> Vec<Action> {
        self.mode = OperationalMode::PowerUp;
        // Nota: el driver real del dueño no espera un power-up unsolicited
        // (manda Go In-Service directo en la traza); el mensaje de power-up
        // queda para la fase 6 cuando se valide contra el host.
        vec![
            Action::Log("power-up".into()),
            Action::ModeChanged(OperationalMode::PowerUp),
        ]
    }

    fn handle_key(&mut self, key: Key) -> Vec<Action> {
        match &self.pending {
            Some(Pending::Digits { target, min, max }) => {
                let (min, max, target) = (*min, *max, *target);
                self.handle_digit_key(key, target, min, max)
            }
            Some(Pending::Pin { max }) => self.handle_pin_key(key, *max),
            _ => Vec::new(),
        }
    }

    fn handle_digit_key(
        &mut self,
        key: Key,
        target: BufferId,
        min: usize,
        max: usize,
    ) -> Vec<Action> {
        let buffer = match target {
            BufferId::Amount => &mut self.buffers.amount,
            BufferId::B => &mut self.buffers.b,
            BufferId::C => &mut self.buffers.c,
        };
        match key {
            Key::Digit(digit) => {
                if buffer.len() < max {
                    buffer.push(digit);
                }
                Vec::new()
            }
            Key::Enter => {
                if buffer.len() >= min {
                    // El estado que pidió la captura decide la transición;
                    // se la notifica vía el intérprete (T3.4).
                    crate::state_exec::on_digits_accepted(self)
                } else {
                    Vec::new()
                }
            }
            Key::Clear => {
                buffer.clear();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn handle_pin_key(&mut self, key: Key, max: usize) -> Vec<Action> {
        match key {
            Key::Digit(digit) => {
                if self.buffers.pin.len() < max {
                    self.buffers.pin.push(digit);
                }
                Vec::new()
            }
            Key::Clear => {
                self.buffers.pin.clear();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn handle_card_inserted(&mut self, card: Card) -> Vec<Action> {
        if self.pending != Some(Pending::Card) {
            return vec![Action::Log(
                "tarjeta insertada fuera de un estado de lectura: ignorada".into(),
            )];
        }
        self.card = Some(card);
        crate::state_exec::on_card_inserted(self)
    }

    fn handle_timer(&mut self, timer: u8) -> Vec<Action> {
        crate::state_exec::on_timer(self, timer)
    }

    // --- helpers ---

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub(crate) fn states(&self) -> &HashMap<StateNumber, StateTableDef> {
        &self.states
    }

    pub(crate) fn states_mut(&mut self) -> &mut HashMap<StateNumber, StateTableDef> {
        &mut self.states
    }

    pub(crate) fn screens(&self) -> &HashMap<ScreenSelector, ScreenDef> {
        &self.screens
    }

    pub(crate) fn screens_mut(&mut self) -> &mut HashMap<ScreenSelector, ScreenDef> {
        &mut self.screens
    }

    pub(crate) fn fits(&self) -> &BTreeMap<u16, FitDef> {
        &self.fits
    }

    pub(crate) fn card(&self) -> Option<&Card> {
        self.card.as_ref()
    }

    pub(crate) fn buffers_mut(&mut self) -> &mut Buffers {
        &mut self.buffers
    }

    pub(crate) fn set_mode(&mut self, mode: OperationalMode) -> Vec<Action> {
        self.mode = mode;
        vec![Action::ModeChanged(mode)]
    }

    pub(crate) fn set_pending(&mut self, pending: Option<Pending>) {
        self.pending = pending;
    }

    pub(crate) fn set_current(&mut self, state: Option<StateNumber>) {
        self.current = state;
    }

    pub(crate) fn set_screen(&mut self, screen: Option<ScreenSelector>) -> Vec<Action> {
        self.screen = screen;
        vec![Action::Display(screen)]
    }

    pub(crate) fn set_config_id(&mut self, id: u16) {
        self.config_id = Some(id);
    }

    pub(crate) fn set_clock(&mut self, clock: DateTimeLoad) {
        self.clock = Some(clock);
    }

    pub(crate) fn set_awaiting_reply(&mut self, awaiting: bool) {
        self.awaiting_reply = awaiting;
    }

    pub(crate) fn awaiting_reply(&self) -> bool {
        self.awaiting_reply
    }

    /// Rota el MCN según el rango configurado (opción 34, §10-27).
    pub(crate) fn next_mcn(&mut self) -> Mcn {
        let extended = self.config.option(34) == Some(1);
        let next = self.mcn.next(extended);
        self.mcn = next;
        next
    }

    /// Mensaje Ready ('9'), o 'B' si la opción 1/001 pide ready separado.
    pub(crate) fn ready(&self, after_reply: bool) -> TerminalMessage {
        let descriptor = if after_reply && self.config.option(1) == Some(1) {
            StatusDescriptor::ReadyAlternate
        } else {
            StatusDescriptor::Ready
        };
        TerminalMessage::SolicitedStatus {
            luno: self.config.luno.clone(),
            descriptor,
            data: Vec::new(),
        }
    }

    /// Command Reject ('A', manual §9-13).
    pub(crate) fn command_reject(&self) -> TerminalMessage {
        TerminalMessage::SolicitedStatus {
            luno: self.config.luno.clone(),
            descriptor: StatusDescriptor::CommandReject,
            data: Vec::new(),
        }
    }

    /// Terminal State ('F') con el identificador g1 del comando respondido
    /// (§9-19) y campos de datos ya formateados.
    pub(crate) fn terminal_state(&self, g1: char, extra: Vec<String>) -> TerminalMessage {
        let mut data = vec![format!(
            "{g1}{}",
            extra.first().cloned().unwrap_or_default()
        )];
        data.extend(extra.into_iter().skip(1));
        TerminalMessage::SolicitedStatus {
            luno: self.config.luno.clone(),
            descriptor: StatusDescriptor::TerminalState,
            data,
        }
    }
}

/// Aplica un comando de customización a los stores (SPEC §4.2).
pub(crate) fn apply_customisation(terminal: &mut Terminal, data: CustomisationData) -> Vec<Action> {
    match data {
        CustomisationData::Screens(screens) => {
            for screen in screens {
                terminal.screens_mut().insert(screen.id, screen);
            }
        }
        CustomisationData::StateTables(tables) => {
            for table in tables {
                terminal.states_mut().insert(table.number, table);
            }
        }
        CustomisationData::Fits(fits) => {
            for fit in fits {
                // BTreeMap por número: la búsqueda respeta el orden (§10-31).
                terminal.fits.insert(fit.number, fit);
            }
        }
        CustomisationData::ConfigParams(ConfigParams {
            options_mask,
            luno,
            timers,
        }) => {
            if let Some(luno) = luno {
                if let Ok(luno) = Luno::new(&luno) {
                    terminal.config_mut().luno = luno;
                }
            }
            let _ = options_mask;
            for timer in timers {
                terminal
                    .config_mut()
                    .timers_cpl
                    .insert(timer.timer, timer.ticks);
            }
        }
        CustomisationData::EnhancedConfig(Ecpl { luno, options }) => {
            if let Some(luno) = luno {
                if let Ok(luno) = Luno::new(&luno) {
                    terminal.config_mut().luno = luno;
                }
            }
            for option in options {
                terminal
                    .config_mut()
                    .options
                    .insert(option.number, option.code);
            }
        }
        CustomisationData::ConfigId(id) => terminal.set_config_id(id),
        CustomisationData::DateTime(clock) => terminal.set_clock(clock),
        CustomisationData::MacFieldSelection(fields) => {
            return vec![Action::Log(format!(
                "MAC Field Selection recibido ({} campos): v1 sin MAC, se ignora",
                fields.len()
            ))];
        }
        CustomisationData::CassetteMapping(fields) => {
            return vec![Action::Log(format!(
                "mapeo de casetes recibido ({} campos): se tipa en la fase de hardware",
                fields.len()
            ))];
        }
        CustomisationData::OverrideReservedScreens(fields) => {
            return vec![Action::Log(format!(
                "override de pantallas reservadas ({} campos): no soportado en v1",
                fields.len()
            ))];
        }
    }
    Vec::new()
}

/// Cierre por defecto ante un next state inválido (manual §2-1): termina la
/// transacción entregando tarjeta; usa el estado 200 si está descargado
/// (SPEC Q10), si no, vuelve a esperar tarjeta.
pub(crate) fn default_close(terminal: &mut Terminal) -> Vec<Action> {
    terminal.set_awaiting_reply(false);
    let close = StateNumber::new(DEFAULT_CLOSE_STATE).expect("200 es válido");
    if terminal.states().contains_key(&close) {
        return crate::state_exec::enter_state(terminal, close);
    }
    terminal.set_pending(None);
    terminal.set_current(None);
    let mut actions = vec![Action::Log(
        "default close: sin estado de cierre descargado; se vuelve a esperar tarjeta".into(),
    )];
    if terminal.card().is_some() {
        actions.push(Action::EjectCard);
    }
    actions
}

/// Acciones comunes al enviar un mensaje (helper para no repetir).
pub(crate) fn send(message: TerminalMessage) -> Action {
    Action::Send(message)
}

/// Acciones comunes para responder Ready tras un mensaje del host.
pub(crate) fn ready_action(terminal: &Terminal, after_reply: bool) -> Action {
    send(terminal.ready(after_reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxi_core::StateNumber;

    #[test]
    fn buffers_default_should_be_zero_and_space_filled() {
        let buffers = Buffers::default();
        assert_eq!(buffers.amount, "00000000");
        assert_eq!(buffers.opcode, [' '; 8]);
        assert!(buffers.pin.is_empty());
    }

    #[test]
    fn reset_for_transaction_should_clear_buffers() {
        let mut buffers = Buffers {
            pin: "1234".into(),
            b: "datos".into(),
            amount: "00000100".into(),
            ..Buffers::default()
        };
        buffers.reset_for_transaction();
        assert!(buffers.pin.is_empty());
        assert!(buffers.b.is_empty());
        assert_eq!(buffers.amount, "00000000");
    }

    #[test]
    fn mcn_should_rotate_and_wrap_at_base_top() {
        let mut terminal = Terminal::new();
        let mut previous = terminal.mcn;
        for _ in 0..40 {
            let next = terminal.next_mcn();
            assert_ne!(next, previous, "el MCN debe avanzar");
            previous = next;
        }
        // Tras 40 rotaciones volvió a pasar por el tope sin salir del rango.
        assert!((0x31..=0x3F).contains(&(previous.as_char() as u32)));
    }

    #[test]
    fn mcn_extended_option_should_allow_wider_range() {
        let mut terminal = Terminal::new();
        terminal.config_mut().options.insert(34, 1);
        let mut seen_above_base = false;
        for _ in 0..40 {
            seen_above_base |= terminal.next_mcn().as_char() as u32 > 0x3F;
        }
        assert!(seen_above_base, "con opción 34 el rango llega a 0x7E");
    }

    #[test]
    fn timer_duration_should_prefer_ecpl_over_cpl_over_default() {
        let mut config = Config::new();
        assert_eq!(config.timer_duration(0), Some(DEFAULT_TIMER));
        config.timers_cpl.insert(0, 10);
        assert_eq!(config.timer_duration(0), Some(Duration::from_secs(8)));
        config.timers_ecpl.insert(0, 5);
        assert_eq!(config.timer_duration(0), Some(Duration::from_secs(5)));
    }

    #[test]
    fn zero_timer_should_mean_infinite() {
        let mut config = Config::new();
        config.timers_ecpl.insert(0, 0);
        assert_eq!(config.timer_duration(0), None);
    }

    #[test]
    fn powered_on_should_set_power_up_mode() {
        let mut terminal = Terminal::new();
        let actions = terminal.handle(Event::PoweredOn);
        assert_eq!(terminal.mode(), OperationalMode::PowerUp);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::ModeChanged(OperationalMode::PowerUp))));
    }

    #[test]
    fn default_close_should_use_state_200_when_loaded() {
        let mut terminal = Terminal::new();
        terminal.states_mut().insert(
            StateNumber::new(200).expect("en rango"),
            StateTableDef {
                number: StateNumber::new(200).expect("en rango"),
                state_type: 'J',
                entries: "052000052052052052052020".into(),
            },
        );
        let actions = default_close(&mut terminal);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Log(message) if message.contains("default close") || message.contains("J"))),
            "debe intentar el estado de cierre 200: {actions:?}"
        );
    }
}
