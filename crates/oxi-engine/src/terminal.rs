//! Terminal NDC: stores de customización, buffers y loop de eventos
//! (SPEC §5; DESIGN §3).
//!
//! El motor es puro y determinista: entra un `Event`, salen `Action`s. No
//! conoce tokio, la red ni la interfaz. Los timers se piden con su número del
//! manual y la duración ya resuelta desde la configuración.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use oxi_core::{
    ConfigParams, CustomisationData, DateTimeLoad, Ecpl, Fdk, FdkMask, FitDef, Luno, Mcn,
    OperationalMode, ScreenDef, ScreenSelector, StateNumber, StateTableDef, StatusDescriptor,
    TerminalMessage, TransactionRequest,
};

use crate::crypto::{CryptoError, SecurityConfig};
use crate::event::{Action, BufferId, Card, Event, Key, Pending};
use crate::hardware::{DeviceCondition, Fault, Hardware};

/// Duración por defecto de un timer no configurado (manual §10-14 Nota 7:
/// 30 ticks de 800 ms ≈ 24 s; aquí se usa 30 s, el default del Cap. 7).
const DEFAULT_TIMER: Duration = Duration::from_secs(30);

/// Estado numero que el terminal usa como cierre por defecto ante un next
/// state inválido (SPEC §14 Q10: el manual no lo define; el driver real usa
/// la batería 200+ para cierres; configurable en el futuro).
const DEFAULT_CLOSE_STATE: u16 = 200;

/// Timer interno para el screen timer de una ITR (manual §10-79: 001-255
/// ticks de 800 ms). 98-99 están reservados por el manual, así que no choca
/// con timers configurables por el host.
pub(crate) const ITR_TIMER: u8 = 98;

/// Buffers de la transacción (manual §2-4: qué inicializa el estado A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffers {
    /// Dígitos de PIN capturados (en claro solo en memoria del simulador).
    pub pin: String,
    /// Bloque de PIN formateado (ISO-0, 16 hex) listo para el TRQ.
    pub pin_block: Option<String>,
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
            pin_block: None,
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
        self.pin_block = None;
        self.b.clear();
        self.c.clear();
        self.amount = "00000000".into();
        self.opcode = [' '; 8];
        self.fdk = None;
    }
}

/// Eco local del campo de entrada en la pantalla.
///
/// Lo consume el runtime para reflejar las teclas sin exponer el PIN en claro:
/// el estado B muestra una 'X' por tecla (manual §2-8), el importe se muestra
/// tal como se digita (§2-17) y el estado H respeta su parámetro de display
/// (§2-23, Tabla 2-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEcho {
    /// Texto a mostrar en el campo de eco (vacío = campo activo sin dígitos).
    pub text: String,
    /// `true` = PIN o datos ocultos ('X' por tecla); `false` = datos visibles.
    pub masked: bool,
}

/// Configuración del terminal que llega del host (CPL/ECPL, SPEC §11).
#[derive(Debug, Clone)]
pub(crate) struct Config {
    luno: Luno,
    /// Opciones del ECPL: número → código (se preservan todas; las no
    /// documentadas se ignoran con warning, SPEC Q13).
    pub(crate) options: HashMap<u8, u16>,
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
    pub(crate) fn option(&self, number: u8) -> Option<u16> {
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
    /// Overrides de pantallas reservadas D0000-D0010 → screen 000-999
    /// (manual §10-55). El manual no publica el mapping D→reservada; se
    /// aplica cuando una pantalla referencia D0000-D0010 (SPEC Q15).
    screen_overrides: BTreeMap<u8, u16>,
    /// Código de idioma 0-9 elegido por S/V (manual §2-50/§2-57).
    language_code: Option<u8>,
    /// Next state por FDK del estado corriente (F/H/X/Y).
    fdk_next: HashMap<char, StateNumber>,
    /// Salida por time-out del estado corriente (se usa en T3.7).
    timeout_next: Option<StateNumber>,
    /// Salida por Cancel del estado corriente.
    cancel_next: Option<StateNumber>,
    /// Bandera de tarjeta del último Transaction Reply (SPEC §4.3).
    card_return: Option<oxi_core::CardReturnFlag>,
    /// MCN enviado en el último TRQ (para validar el reply, §9-2).
    pending_mcn: Option<char>,
    /// Estado al que volver si el cardholder reintenta tras el time-out.
    retry_state: Option<StateNumber>,
    /// Última transacción para el status data del TRQ (opción 15, §9-2 q/r).
    last_transaction: Option<LastTransaction>,
    /// Next state pendiente de completar el dispensado (function 2/8, §10-58:
    /// la transición espera al retiro del efectivo o al timer 07).
    post_cash_next: Option<StateNumber>,
    /// Hardware virtual (dispensador, lector, impresoras, sensores).
    hardware: Hardware,
    /// Intentos de transacción detectados (campo g3 de Tabla 9-8).
    attempts: u32,
    /// Configuración de seguridad (claves de PRUEBA, fase 7).
    security: SecurityConfig,
    /// Guarda contra recursión del cierre por defecto (cadena J→000→…).
    closing: bool,
}

/// Resumen de la última transacción (Tabla 9-1 q/r).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LastTransaction {
    /// Serial de 4 dígitos.
    pub serial: u16,
    /// '0' ninguna, '1' terminación buena, '2' error, '3' reply rechazado.
    pub status: char,
    /// Billetes dispensados por casete (4 × 5 dígitos).
    pub notes: [u16; 4],
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
            screen_overrides: BTreeMap::new(),
            language_code: None,
            fdk_next: HashMap::new(),
            timeout_next: None,
            cancel_next: None,
            card_return: None,
            pending_mcn: None,
            retry_state: None,
            last_transaction: None,
            post_cash_next: None,
            hardware: Hardware::default(),
            attempts: 0,
            security: SecurityConfig::default(),
            closing: false,
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

    /// Eco local del campo de entrada en curso (ver [`InputEcho`]).
    pub fn input_echo(&self) -> Option<InputEcho> {
        match &self.pending {
            Some(Pending::Itr { response }) => match response.display_flag {
                // 0 = no mostrar la entrada (§10-79).
                '0' => None,
                // 2 = mostrar '*' por cada tecla (§10-79).
                '2' => Some(InputEcho {
                    text: "*".repeat(self.buffers.b.chars().count().min(30)),
                    masked: true,
                }),
                // 1 = mostrar los datos tal cual.
                _ => Some(InputEcho {
                    text: self.buffers.b.clone(),
                    masked: false,
                }),
            },
            Some(Pending::Pin { .. }) => Some(InputEcho {
                text: "X".repeat(self.buffers.pin.chars().count().min(30)),
                masked: true,
            }),
            Some(Pending::Digits { target, masked, .. }) => {
                let raw = match target {
                    BufferId::Amount => self.buffers.amount.trim_start_matches('0').to_owned(),
                    BufferId::B => self.buffers.b.clone(),
                    BufferId::C => self.buffers.c.clone(),
                };
                if *masked {
                    Some(InputEcho {
                        text: "X".repeat(raw.chars().count().min(30)),
                        masked: true,
                    })
                } else {
                    Some(InputEcho {
                        text: raw,
                        masked: false,
                    })
                }
            }
            _ => None,
        }
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

    /// Contenido crudo de una pantalla descargada (el runtime lo renderiza).
    pub fn screen_content(&self, id: &ScreenSelector) -> Option<&[u8]> {
        self.screens.get(id).map(|screen| screen.data.as_slice())
    }

    // --- loop de eventos ---

    /// Procesa un evento y devuelve las acciones a ejecutar.
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::PoweredOn => self.handle_powered_on(),
            Event::HostMessage(message) => crate::host_ops::handle_host(self, message),
            Event::HostParseError { reject } => crate::host_ops::handle_parse_error(self, &reject),
            Event::KeyPressed(key) => self.handle_key(key),
            Event::CardInserted(card) => self.handle_card_inserted(card),
            Event::CardTaken => {
                self.card = None;
                self.hardware.reader.state = Some(crate::hardware::ReaderState::Idle);
                vec![
                    Action::Log("tarjeta retirada por el cardholder".into()),
                    Action::CancelTimer(9),
                ]
            }
            Event::CashTaken => crate::state_exec::on_cash_taken(self),
            Event::TimerFired(timer) => self.handle_timer(timer),
        }
    }

    fn handle_powered_on(&mut self) -> Vec<Action> {
        // Tras el power-up el terminal queda fuera de servicio hasta que el
        // host lo ponga en servicio (la columna Power-Up de las Tablas
        // 10-38/40 es la ventana previa, sin respuestas).
        self.mode = OperationalMode::OutOfService;
        vec![
            Action::Log("power-up: terminal fuera de servicio".into()),
            Action::ModeChanged(OperationalMode::OutOfService),
        ]
    }

    fn handle_key(&mut self, key: Key) -> Vec<Action> {
        // ITR: el host definió teclas activas y buffer B; no pasa por las
        // salidas del estado I ni por las máscaras de state table (§10-79).
        if matches!(self.pending, Some(Pending::Itr { .. })) {
            return self.handle_itr_key(key);
        }
        if key == Key::Cancel {
            return crate::state_exec::on_cancel(self);
        }
        // FDK: si el estado corriente le asignó un next state, transiciona
        // (los estados X/Y además escriben buffers antes: on_fdk_pressed).
        if let Key::Fdk(fdk) = key {
            return match self.fdk_next_for(fdk) {
                Some(next) => crate::state_exec::on_fdk_pressed(self, fdk, next),
                None => Vec::new(),
            };
        }
        // Enter equivale a FDK 'A' (manual §2-17, §2-23).
        if key == Key::Enter {
            return match self.fdk_next_for(Fdk::A) {
                Some(next) => crate::state_exec::on_fdk_pressed(self, Fdk::A, next),
                None => Vec::new(),
            };
        }
        match &self.pending {
            Some(Pending::Digits { target, max, .. }) => {
                let (max, target) = (*max, *target);
                match target {
                    BufferId::Amount => {
                        // El importe se mantiene rellenado a ceros; cada
                        // dígito entra por la derecha (shift, §2-17).
                        let width = self.buffers.amount.len();
                        match key {
                            Key::Digit(digit) => {
                                let mut digits =
                                    self.buffers.amount.trim_start_matches('0').to_owned();
                                digits.push(digit);
                                if digits.len() > max {
                                    digits = digits[digits.len() - max..].to_owned();
                                }
                                self.buffers.amount = format!("{digits:0>width$}");
                            }
                            Key::Clear => self.buffers.amount = "0".repeat(width),
                            _ => {}
                        }
                    }
                    BufferId::B | BufferId::C => {
                        let buffer = match target {
                            BufferId::B => &mut self.buffers.b,
                            _ => &mut self.buffers.c,
                        };
                        match key {
                            Key::Digit(digit) => {
                                if buffer.len() < max {
                                    buffer.push(digit);
                                }
                            }
                            Key::Clear => buffer.clear(),
                            _ => {}
                        }
                    }
                }
                Vec::new()
            }
            Some(Pending::Pin { max }) => {
                let max = *max;
                match key {
                    Key::Digit(digit) => {
                        if self.buffers.pin.len() < max {
                            self.buffers.pin.push(digit);
                        }
                    }
                    Key::Clear => self.buffers.pin.clear(),
                    _ => {}
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Teclas durante una ITR: sólo las activas por el host; los datos van a
    /// buffer B y el último char es 'E' (Cancel) o 'T' (timeout) (§9-2 m).
    fn handle_itr_key(&mut self, key: Key) -> Vec<Action> {
        let Some(Pending::Itr { response }) = self.pending.clone() else {
            return Vec::new();
        };
        match key {
            Key::Digit(digit) => {
                if response.numeric_active() && self.buffers.b.len() < 32 {
                    self.buffers.b.push(digit);
                }
                Vec::new()
            }
            Key::Clear => {
                self.buffers.b.clear();
                Vec::new()
            }
            Key::Enter if response.enter_active() => self.finish_itr(None),
            Key::Fdk(Fdk::A) if response.fdk_active(Fdk::A) => self.finish_itr(None),
            Key::Fdk(fdk) if response.fdk_active(fdk) => {
                // Convención del simulador (SPEC Q14): la FDK seleccionada se
                // representa por su letra A-I, con ceros a la izquierda.
                self.buffers.b = fdk.as_char().to_string();
                self.finish_itr(None)
            }
            Key::Cancel if response.cancel_active() => self.finish_itr(Some('E')),
            _ => Vec::new(),
        }
    }

    /// Cierra la ITR y envía el TRQ abreviado de Tabla 9-2 (sólo buffer B).
    fn finish_itr(&mut self, terminator: Option<char>) -> Vec<Action> {
        if !matches!(self.pending, Some(Pending::Itr { .. })) {
            return Vec::new();
        }
        let Some(mcn_char) = self.pending_mcn else {
            return vec![Action::Log("ITR sin MCN pendiente: no se responde".into())];
        };
        let Ok(mcn) = Mcn::new(mcn_char, true) else {
            return vec![Action::Log(format!(
                "ITR con MCN inválido '{mcn_char}': no se responde"
            ))];
        };

        let mut data = self.buffers.b.clone();
        if let Some(terminator) = terminator {
            // El máximo del buffer B es 32; dejamos lugar al terminador.
            data.truncate(31);
            data.push(terminator);
        }
        if data.len() < 3 {
            data = format!("{data:0>3}");
        }
        self.buffers.b = data.clone();

        // Tabla 9-2: clase 1/1 abreviada, sólo buffer B (el resto vacío).
        let request = TransactionRequest {
            luno: self.luno().clone(),
            tvn: None,
            top_of_receipt: '1',
            mcn,
            track2: None,
            track3: None,
            op_code: None,
            amount: None,
            buffer_a: None,
            buffer_b: Some(data),
            buffer_c: None,
            track1: None,
            status_data: None,
        };
        if let Err(err) = request.to_bytes() {
            return vec![Action::Log(format!("TRQ de ITR inválido: {err}"))];
        }

        self.pending = Some(Pending::TransactionReply);
        self.awaiting_reply = true;
        vec![
            Action::CancelTimer(ITR_TIMER),
            Action::CancelTimer(3),
            Action::Send(TerminalMessage::TransactionRequest(Box::new(request))),
            Action::StartTimer {
                timer: 3,
                after: self.config.timer_duration(3),
            },
            Action::Log("ITR: respuesta abreviada enviada al host".into()),
        ]
    }

    /// Timer de pantalla ITR vencido: buffer B + 'T' y TRQ abreviado.
    pub(crate) fn itr_timeout(&mut self) -> Vec<Action> {
        if !matches!(self.pending, Some(Pending::Itr { .. })) {
            return vec![Action::Log("timer 98 sin ITR pendiente".into())];
        }
        let mut actions = vec![Action::Log(
            "ITR: timer de pantalla vencido; se envía 'T'".into(),
        )];
        actions.extend(self.finish_itr(Some('T')));
        actions
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

    #[cfg(test)]
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

    pub(crate) fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    pub(crate) fn set_current(&mut self, state: Option<StateNumber>) {
        self.current = state;
    }

    pub(crate) fn set_screen(&mut self, screen: Option<ScreenSelector>) -> Vec<Action> {
        let screen = screen.map(|selector| self.resolve_screen_override(selector));
        self.screen = screen;
        vec![Action::Display(screen)]
    }

    /// Aplica un override D0000-D0010 si la pantalla lo referencia (§10-55).
    fn resolve_screen_override(&self, screen: ScreenSelector) -> ScreenSelector {
        let ScreenSelector::Group {
            letter: 'D',
            digits,
        } = screen
        else {
            return screen;
        };
        if digits > 10 {
            return screen;
        }
        let Some(user) = self.screen_overrides.get(&(digits as u8)) else {
            return screen;
        };
        oxi_core::ScreenId::new(*user)
            .map(ScreenSelector::Customer)
            .unwrap_or(screen)
    }

    /// Código de idioma seleccionado (0-9), si S/V ya lo fijó.
    pub fn language_code(&self) -> Option<u8> {
        self.language_code
    }

    pub(crate) fn set_language_code(&mut self, code: Option<u8>) {
        self.language_code = code;
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

    /// ¿Hay una transacción esperando Transaction Reply/ITR? (Tabla 10-39).
    pub fn awaiting_reply(&self) -> bool {
        self.awaiting_reply
    }

    /// Limpia la configuración de teclado al cambiar de estado.
    pub(crate) fn clear_state_keyboard(&mut self) {
        self.fdk_next.clear();
        self.timeout_next = None;
        self.cancel_next = None;
    }

    /// Asigna el next state por FDK desde las entradas `first..first+4`
    /// (F/H: entradas 5-8 = FDK A-D; 255 desactiva).
    pub(crate) fn set_fdk_next_from_entries(&mut self, entries: &str, first: usize) {
        for (offset, fdk) in [Fdk::A, Fdk::B, Fdk::C, Fdk::D].into_iter().enumerate() {
            let text = entries
                .get((first - 2 + offset) * 3..(first - 2 + offset) * 3 + 3)
                .and_then(|t| t.parse::<u16>().ok());
            if let Some(value) = text {
                if value != 255 && value <= 999 {
                    if let Ok(state) = oxi_core::StateNumber::new(value) {
                        self.fdk_next.insert(fdk.as_char(), state);
                    }
                }
            }
        }
    }

    /// Asigna el mismo next state a todas las FDK activas de la máscara
    /// (X/Y: cualquier FDK lleva al mismo estado, §2-59/§2-62).
    pub(crate) fn set_fdk_next_uniform(&mut self, next: StateNumber, mask: FdkMask) {
        for fdk in [
            Fdk::A,
            Fdk::B,
            Fdk::C,
            Fdk::D,
            Fdk::F,
            Fdk::G,
            Fdk::H,
            Fdk::I,
        ] {
            if mask.is_active(fdk) {
                self.fdk_next.insert(fdk.as_char(), next);
            }
        }
    }

    /// Next state de una FDK (si el estado la tiene activa).
    pub(crate) fn fdk_next_for(&self, fdk: Fdk) -> Option<StateNumber> {
        self.fdk_next.get(&fdk.as_char()).copied()
    }

    pub(crate) fn set_timeout_next(&mut self, next: Option<StateNumber>) {
        self.timeout_next = next;
    }

    pub(crate) fn timeout_next(&self) -> Option<StateNumber> {
        self.timeout_next
    }

    /// Asigna (o borra) el next state de una FDK concreta.
    pub(crate) fn set_fdk_next(&mut self, fdk: Fdk, next: Option<StateNumber>) {
        match next {
            Some(state) => {
                self.fdk_next.insert(fdk.as_char(), state);
            }
            None => {
                self.fdk_next.remove(&fdk.as_char());
            }
        }
    }

    pub(crate) fn set_cancel_next(&mut self, next: Option<StateNumber>) {
        self.cancel_next = next;
    }

    pub(crate) fn cancel_next(&self) -> Option<StateNumber> {
        self.cancel_next
    }

    /// Bandera de tarjeta del último Transaction Reply.
    pub(crate) fn card_return(&self) -> Option<oxi_core::CardReturnFlag> {
        self.card_return
    }

    /// Rechazo específico ('C') con código valor/calificador (§10-88); si la
    /// opción 12 no está activa, cae a Command Reject ('A').
    pub(crate) fn specific_command_reject(&self, code: &str) -> TerminalMessage {
        if self.config.option(12) == Some(1) {
            TerminalMessage::SolicitedStatus {
                luno: self.config.luno.clone(),
                descriptor: StatusDescriptor::SpecificCommandReject,
                data: vec![code.to_owned()],
            }
        } else {
            self.command_reject()
        }
    }

    /// Limpia el MCN pendiente al cerrar la espera del reply.
    pub(crate) fn cancel_pending_mcn(&mut self) {
        self.pending_mcn = None;
    }

    pub(crate) fn set_card_return(&mut self, flag: Option<oxi_core::CardReturnFlag>) {
        self.card_return = flag;
    }

    /// La tarjeta fue retenida/entregada al dispositivo.
    pub(crate) fn take_card(&mut self) {
        self.card = None;
    }

    /// ¿Está en curso una cadena de cierre por defecto? (guarda anti-recursión)
    pub(crate) fn is_closing(&self) -> bool {
        self.closing
    }

    pub(crate) fn set_closing(&mut self, closing: bool) {
        self.closing = closing;
    }

    pub(crate) fn set_pending_mcn(&mut self, mcn: char) {
        self.pending_mcn = Some(mcn);
    }

    pub(crate) fn pending_mcn(&self) -> Option<char> {
        self.pending_mcn
    }

    pub(crate) fn set_retry_state(&mut self, state: Option<StateNumber>) {
        self.retry_state = state;
    }

    pub(crate) fn retry_state(&self) -> Option<StateNumber> {
        self.retry_state
    }

    pub(crate) fn set_last_transaction(&mut self, last: LastTransaction) {
        self.last_transaction = Some(last);
    }

    pub(crate) fn last_transaction(&self) -> Option<LastTransaction> {
        self.last_transaction
    }

    /// Guarda el next state que se ejecutará al completar el dispensado.
    pub(crate) fn set_post_cash_next(&mut self, next: Option<StateNumber>) {
        self.post_cash_next = next;
    }

    /// Toma (y limpia) el next state pendiente del dispensado.
    pub(crate) fn take_post_cash_next(&mut self) -> Option<StateNumber> {
        self.post_cash_next.take()
    }

    pub(crate) fn hardware(&self) -> &Hardware {
        &self.hardware
    }

    pub(crate) fn hardware_mut(&mut self) -> &mut Hardware {
        &mut self.hardware
    }

    /// Configuración de seguridad (claves de prueba del perfil).
    pub fn security(&self) -> &SecurityConfig {
        &self.security
    }

    /// Acceso mutable a la seguridad (cambio de claves en runtime).
    pub(crate) fn security_mut(&mut self) -> &mut SecurityConfig {
        &mut self.security
    }

    /// Aplica la configuración de seguridad del perfil (fase 7).
    pub fn configure_security(&mut self, security: SecurityConfig) {
        self.security = security;
    }

    /// Forma el PIN block ISO-0 y lo cifra si hay clave configurada.
    /// Devuelve el bloque listo para el TRQ o el error de formación.
    pub(crate) fn build_pin_block(&self, pin: &str, pan: &str) -> Result<String, CryptoError> {
        let iso0 = crate::crypto::iso0_pin_block(pin, pan)?;
        match self.security.working_pin_key() {
            None => Ok(iso0),
            Some(Err(err)) => Err(err),
            Some(Ok(key)) => {
                crate::crypto::encrypt_pin_block(&iso0, &key, self.security.pin_key_mode)
            }
        }
    }

    /// KVV de la clave de trabajo actual (6 hex, §11-18), si hay clave.
    pub fn pin_kvv(&self) -> Option<Result<String, CryptoError>> {
        let key = self.security.working_pin_key()?;
        Some(key.and_then(|key| crate::crypto::kvv(&key, self.security.pin_key_mode)))
    }

    /// Cuenta un intento de transacción (campo g3 de la Tabla 9-8).
    pub(crate) fn count_attempt(&mut self) {
        self.attempts = self.attempts.wrapping_add(1).min(9_999_999);
    }

    /// Payload del Send Supply Counters (Tabla 9-8): TSN(4) + acumuladas(7)
    /// + 4×20 contadores + capturadas(5) + sobres(5) + cámara(5) + serial(5).
    pub(crate) fn supply_counters_payload(&self) -> String {
        let tsn = self.last_transaction.map(|last| last.serial).unwrap_or(0);
        let dispenser = &self.hardware.dispenser;
        format!(
            "{tsn:04}{:07}{}{}{}{}{:05}{:05}0000000000",
            self.attempts,
            dispenser.notes_in_cassettes(),
            dispenser.notes_rejected(),
            dispenser.notes_dispensed(),
            dispenser.last_notes_dispensed(),
            self.hardware.reader.captures.min(99999),
            0, // sobres depositados (sin depository en v1)
        )
    }

    /// Aplica una falla de hardware y devuelve las acciones (unsolicited
    /// status cuando la condición lo amerita, §9-58).
    pub fn inject_fault(&mut self, fault: Fault) -> Vec<Action> {
        let config_id = self.config_id.unwrap_or(0);
        let mut actions = vec![Action::Log(format!("falla inyectada: {fault:?}"))];
        if let Some(condition) = self.hardware.inject(fault, config_id) {
            actions.push(Action::Send(self.unsolicited(condition)));
        }
        actions
    }

    /// Entra/sale de modo supervisor (pantalla local): reporta 'P2x'
    /// (Tabla 9-41) como unsolicited.
    pub fn set_supervisor_mode(&mut self, active: bool) -> Vec<Action> {
        self.hardware.sensors.supervisor_mode = active;
        let field = self.hardware.sensors.mode_change_field();
        vec![
            Action::Log(format!("modo supervisor: {active}")),
            Action::Send(TerminalMessage::UnsolicitedStatus {
                luno: self.config.luno.clone(),
                data: vec![field],
            }),
        ]
    }

    /// Unsolicited status a partir de una condición de dispositivo
    /// (Tabla 9-27: DIG+status, severidad, diagnóstico, supplies).
    pub(crate) fn unsolicited(&self, condition: DeviceCondition) -> TerminalMessage {
        TerminalMessage::UnsolicitedStatus {
            luno: self.config.luno.clone(),
            data: condition.status_fields(),
        }
    }

    /// Solicited device fault ('8', §9-13): respuesta cuando una operación
    /// del Transaction Reply falla y Central debe enterarse.
    pub(crate) fn device_fault(&self, condition: DeviceCondition) -> TerminalMessage {
        TerminalMessage::SolicitedStatus {
            luno: self.config.luno.clone(),
            descriptor: StatusDescriptor::DeviceFault,
            data: condition.status_fields(),
        }
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
        CustomisationData::OverrideReservedScreens(overrides) => {
            let count = overrides.len();
            for over in overrides {
                match over.user_screen {
                    Some(user) => {
                        terminal.screen_overrides.insert(over.reserved, user);
                    }
                    None => {
                        terminal.screen_overrides.remove(&over.reserved);
                    }
                }
            }
            return vec![Action::Log(format!(
                "override de pantallas reservadas: {count} entrada(s)"
            ))];
        }
    }
    Vec::new()
}

/// Cierre por defecto ante un next state inválido (manual §2-1): termina la
/// transacción entregando tarjeta; usa el estado 200 si está descargado
/// (SPEC Q10), si no, vuelve a esperar tarjeta.
///
/// Guarda anti-recursión: si una cadena de cierre vuelve a caer acá (p.ej.
/// J 200 → next 000 inexistente), se detiene en idle en vez de desbordar la
/// pila.
pub(crate) fn default_close(terminal: &mut Terminal) -> Vec<Action> {
    terminal.set_awaiting_reply(false);
    if terminal.is_closing() {
        terminal.set_pending(None);
        terminal.set_current(None);
        return vec![Action::Log(
            "default close recursivo: se detiene en idle".into(),
        )];
    }
    terminal.set_closing(true);
    let close = StateNumber::new(DEFAULT_CLOSE_STATE).expect("200 es válido");
    let mut actions = if terminal.states().contains_key(&close) {
        crate::state_exec::enter_state(terminal, close)
    } else {
        terminal.set_pending(None);
        terminal.set_current(None);
        let mut fallback = vec![Action::Log(
            "default close: sin estado de cierre descargado; se vuelve a esperar tarjeta".into(),
        )];
        if terminal.card().is_some() {
            fallback.push(Action::EjectCard);
        }
        fallback
    };
    terminal.set_closing(false);
    if actions.is_empty() {
        actions.push(Action::Log("default close aplicado".into()));
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
    use oxi_core::{ScreenOverride, StateNumber};

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
    fn powered_on_should_go_out_of_service() {
        let mut terminal = Terminal::new();
        let actions = terminal.handle(Event::PoweredOn);
        assert_eq!(terminal.mode(), OperationalMode::OutOfService);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::ModeChanged(OperationalMode::OutOfService))));
    }

    #[test]
    fn default_close_should_use_state_200_when_loaded() {
        let mut terminal = Terminal::new();
        terminal.states_mut().insert(
            StateNumber::new(200).expect("en rango"),
            StateTableDef {
                number: StateNumber::new(200).expect("en rango"),
                state_type: 'J',
                // receipt 052, next 000, resto 052
                entries: "052000052052052052052000".into(),
            },
        );
        let actions = default_close(&mut terminal);
        // El J del 200 mostró su pantalla de recibo (052): el cierre ejecutó.
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::Display(Some(ScreenSelector::Customer(id))) if id.value() == 52
            )),
            "debe ejecutar el estado de cierre 200: {actions:?}"
        );
    }

    #[test]
    fn override_d0000_should_replace_the_displayed_screen() {
        let mut terminal = Terminal::new();
        let _ = apply_customisation(
            &mut terminal,
            CustomisationData::OverrideReservedScreens(vec![ScreenOverride {
                reserved: 0,
                user_screen: Some(25),
            }]),
        );
        let actions = terminal.set_screen(Some(ScreenSelector::Group {
            letter: 'D',
            digits: 0,
        }));
        let expected = ScreenSelector::Customer(oxi_core::ScreenId::new(25).expect("en rango"));
        assert_eq!(terminal.screen(), Some(expected));
        assert!(matches!(
            actions.as_slice(),
            [Action::Display(Some(ScreenSelector::Customer(id)))] if id.value() == 25
        ));
    }

    #[test]
    fn override_without_value_should_revert_to_reserved_reference() {
        let mut terminal = Terminal::new();
        let _ = apply_customisation(
            &mut terminal,
            CustomisationData::OverrideReservedScreens(vec![ScreenOverride {
                reserved: 0,
                user_screen: None,
            }]),
        );
        let reference = ScreenSelector::Group {
            letter: 'D',
            digits: 0,
        };
        let _ = terminal.set_screen(Some(reference));
        assert_eq!(terminal.screen(), Some(reference));
    }
}
