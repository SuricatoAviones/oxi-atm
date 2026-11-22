//! Intérprete de state tables (SPEC §5; manual Cap. 2).
//!
//! Layouts de entradas verificados contra las tablas del manual (2-10 para F,
//! 2-14 para H, etc.) y contra el parser de `atm-states` de electron-atm que
//! operó contra el driver real del dueño.
//!
//! Tipos implementados: A, D, F, H, J, X, Y, W. K/B llegan en T3.6 e I en
//! T3.5.

use oxi_core::{Fdk, FdkMask, ScreenId, ScreenSelector, StateNumber, StateTableDef};

use crate::event::{Action, BufferId, Pending};
use crate::terminal::{default_close, Terminal};

/// Entrada `index` (numeración del manual: 1 = tipo, 2 = primera entrada de
/// datos) como texto de 3 chars.
fn entry(entries: &str, index: usize) -> Option<&str> {
    let start = (index - 2) * 3;
    entries.get(start..start + 3)
}

/// Entrada como número de estado; 255 → `None` (desactivado).
fn entry_state(entries: &str, index: usize) -> Option<StateNumber> {
    let text = entry(entries, index)?;
    let value: u16 = text.parse().ok()?;
    if value == 255 || value > 999 {
        return None;
    }
    StateNumber::new(value).ok()
}

/// Entrada como pantalla de cardholder (000-999).
fn entry_screen(entries: &str, index: usize) -> Option<ScreenSelector> {
    let text = entry(entries, index)?;
    let value: u16 = text.parse().ok()?;
    ScreenId::new(value).ok().map(ScreenSelector::Customer)
}

/// Entrada como máscara decimal (000-255).
fn entry_mask(entries: &str, index: usize) -> Option<u8> {
    entry(entries, index)?.parse().ok()
}

/// Entra al estado `number` y ejecuta el comportamiento de su tipo.
pub(crate) fn enter_state(terminal: &mut Terminal, number: StateNumber) -> Vec<Action> {
    terminal.set_current(Some(number));
    terminal.clear_state_keyboard();
    let Some(table) = terminal.states().get(&number).cloned() else {
        return default_close(terminal);
    };
    match table.state_type {
        'A' => state_card_read(terminal, &table),
        'B' => state_pin_entry(terminal, &table),
        'D' => state_preset_opcode(terminal, &table),
        'F' => state_amount_entry(terminal, &table),
        'H' => state_info_entry(terminal, &table),
        'I' => state_transaction_request(terminal, &table),
        'J' => state_close(terminal, &table),
        'K' => state_fit_switch(terminal, &table),
        'X' => state_fdk_info(terminal, &table),
        'Y' => state_eight_fdk(terminal, &table),
        'W' => state_fdk_switch(terminal, &table),
        // '_' (FIT switch ampliado), M, R, S, T, V, b, k, m, > y w llegan en
        // fases posteriores (ROADMAP).
        other => vec![
            Action::Log(format!(
                "estado {number} tipo '{other}': no implementado todavía"
            )),
            Action::Log("se aplica default close".into()),
        ]
        .into_iter()
        .chain(default_close(terminal))
        .collect(),
    }
}

/// Hook: tarjeta insertada mientras un estado A espera.
pub(crate) fn on_card_inserted(terminal: &mut Terminal) -> Vec<Action> {
    let Some(current) = terminal.current_state() else {
        return vec![Action::Log("tarjeta sin estado de lectura".into())];
    };
    let Some(table) = terminal.states().get(&current).cloned() else {
        return default_close(terminal);
    };
    if table.state_type != 'A' {
        return vec![Action::Log(format!(
            "estado {current} no es de lectura de tarjeta: tarjeta ignorada"
        ))];
    }
    let entries = &table.entries;
    let card = terminal.card().cloned().unwrap_or_default();
    // Fallas del lector: un atasco o error de lectura no produce lectura
    // válida (device 'D', Tabla 9-30).
    if terminal.hardware().reader.jammed || terminal.hardware().reader.read_error {
        let mut actions = vec![Action::Log(
            "lector con falla: la tarjeta no se pudo leer".into(),
        )];
        if let Some(error_screen) = entry_screen(entries, 4) {
            actions.extend(terminal.set_screen(Some(error_screen)));
        }
        if !terminal.hardware().reader.jammed {
            actions.push(Action::EjectCard);
        }
        if let Some(next) = entry_state(entries, 9) {
            actions.extend(enter_state(terminal, next));
        } else {
            actions.extend(default_close(terminal));
        }
        return actions;
    }
    // Condiciones de lectura en orden (§2-6): bit0 pista 3, bit1 pista 2,
    // bit2 pista 1, bit3 chip (sin soporte en v1).
    for condition_entry in [5usize, 6, 7] {
        let Some(raw) = entry_mask(entries, condition_entry) else {
            continue;
        };
        if raw == 255 {
            continue;
        }
        let wants_track3 = raw & 0b0001 != 0;
        let wants_track2 = raw & 0b0010 != 0;
        let wants_track1 = raw & 0b0100 != 0;
        let satisfied = (!wants_track3 || card.track3.is_some())
            && (!wants_track2 || card.track2.is_some())
            && (!wants_track1 || card.track1.is_some());
        if satisfied {
            if let Some(next) = entry_state(entries, 3) {
                return enter_state(terminal, next);
            }
        }
    }
    // Ninguna condición: pantalla de error, expulsión y salida no-FIT (§2-4).
    let mut actions = Vec::new();
    if let Some(error_screen) = entry_screen(entries, 4) {
        actions.extend(terminal.set_screen(Some(error_screen)));
    }
    actions.push(Action::EjectCard);
    if let Some(next) = entry_state(entries, 9) {
        actions.extend(enter_state(terminal, next));
    } else {
        actions.extend(default_close(terminal));
    }
    actions
}

/// Hook: FDK pulsada con next state resuelto por el estado actual.
pub(crate) fn on_fdk_pressed(terminal: &mut Terminal, fdk: Fdk, next: StateNumber) -> Vec<Action> {
    let Some(current) = terminal.current_state() else {
        return enter_state(terminal, next);
    };
    let state_type = terminal
        .states()
        .get(&current)
        .map(|table| table.state_type);
    let mut actions = Vec::new();
    match state_type {
        // Y: guarda el keycode en el buffer de FDK y escribe el op-code en
        // las posiciones de la máscara (§2-62).
        Some('Y') => {
            terminal.buffers_mut().fdk = Some(fdk.as_char());
            if let Some(table) = terminal.states().get(&current).cloned() {
                let positions = entry_mask(&table.entries, 7).unwrap_or(0);
                if positions != 255 {
                    for bit in 0..8u8 {
                        if positions & (1 << bit) != 0 {
                            terminal.buffers_mut().opcode[bit as usize] = fdk.as_char();
                        }
                    }
                }
            }
        }
        // X: busca el valor de esta FDK en la extensión Z y lo escribe en el
        // buffer configurado (§2-61): 01X buffer B, 02X C, 03X importe.
        Some('X') => {
            if let Some(table) = terminal.states().get(&current).cloned() {
                apply_fdk_info_value(terminal, &table, fdk);
            }
        }
        // B: valida el PIN remoto formando el bloque ISO-0 antes de seguir
        // (manual §11-3; el FIT del driver real pide verificación remota).
        Some('B') => {
            let pin = terminal.buffers().pin.clone();
            let pan = terminal
                .card()
                .and_then(|card| card.track2.clone())
                .unwrap_or_default();
            match crate::crypto::iso0_pin_block(&pin, &pan) {
                Ok(block) => terminal.buffers_mut().pin_block = Some(block),
                Err(err) => actions.push(Action::Log(format!(
                    "PIN block no formado: {err}; el TRQ irá sin buffer A"
                ))),
            }
        }
        _ => {}
    }
    actions.extend(enter_state(terminal, next));
    actions
}

/// FIT 匹配 por FIID (manual §8-1, §8-2): primera FIT cuyo PFIID coincide
/// con los bytes de la pista 2 en el offset PIDDX; 0xFF es comodín.
fn fit_match(terminal: &Terminal) -> Option<&oxi_core::FitDef> {
    let track2 = terminal.card()?.track2.as_ref()?;
    let bytes = track2.as_bytes();
    terminal.fits().values().find(|fit| {
        let Some(&pidx) = fit.entries.first() else {
            return false;
        };
        let pidx = pidx as usize;
        let Some(pfiid) = fit.entries.get(1..6) else {
            return false;
        };
        pidx + 5 <= bytes.len()
            && pfiid
                .iter()
                .enumerate()
                .all(|(offset, &byte)| byte == 0xFF || byte == bytes[pidx + offset])
    })
}

/// K - FIT Switch (§2-35): conmuta por el índice de la FIT que matchea.
fn state_fit_switch(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let index = terminal
        .fits()
        .values()
        .position(|fit| fit_match(terminal).is_some_and(|matched| std::ptr::eq(matched, fit)));
    match index {
        Some(index) => match entry_state(&table.entries, index + 2) {
            Some(next) => enter_state(terminal, next),
            None => default_close(terminal),
        },
        None => vec![Action::Log(
            "FIT switch: ninguna FIT coincide con la tarjeta".into(),
        )]
        .into_iter()
        .chain(default_close(terminal))
        .collect(),
    }
}

/// B - PIN Entry (§2-8) con verificación remota (FIT PCKLN=0 del driver).
fn state_pin_entry(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let max_pin = fit_match(terminal)
        .and_then(|fit| fit.entries.get(8).copied())
        .map_or(16, usize::from)
        .clamp(4, 16);
    terminal.set_pending(Some(Pending::Pin { max: max_pin }));
    // Enter = FDK A → remote pin check next state (entrada 8).
    terminal.set_fdk_next(Fdk::A, entry_state(&table.entries, 8));
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.set_cancel_next(entry_state(&table.entries, 4));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions.push(Action::StartTimer {
        timer: 0,
        after: terminal.config().timer_duration(0),
    });
    actions
}

/// I - Transaction Request (§2-25): arma y envía el TRQ; luego espera reply.
fn state_transaction_request(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    let send = |index: usize| entry(&table.entries, index) == Some("001");

    let card = terminal.card().cloned().unwrap_or_default();
    let track2 = send(4).then_some(card.track2).flatten();
    let track1 = send(5).then_some(card.track1).flatten();
    let track3 = send(5).then_some(card.track3).flatten();
    let op_code = send(6).then(|| terminal.buffers().opcode.iter().collect::<String>());
    let amount = send(7).then(|| terminal.buffers().amount.clone());
    let pin_requested = matches!(entry(&table.entries, 8), Some("001" | "129"));
    let buffer_a = pin_requested
        .then(|| terminal.buffers().pin_block.clone())
        .flatten();
    let buffers = entry(&table.entries, 9).unwrap_or("000");
    let buffer_b = matches!(buffers, "001" | "003").then(|| terminal.buffers().b.clone());
    let buffer_c = matches!(buffers, "002" | "003").then(|| terminal.buffers().c.clone());

    // Status data de la transacción anterior (opción 15, Tabla 9-1 q/r).
    let status_data = if terminal.config().option(15) == Some(1) {
        Some(match terminal.last_transaction() {
            Some(last) => {
                let notes: String = last
                    .notes
                    .iter()
                    .map(|count| format!("{count:05}"))
                    .collect();
                format!("{:04}{}{}", last.serial, last.status, notes)
            }
            None => format!("0000{}", "0".repeat(21)),
        })
    } else {
        None
    };

    let mcn = terminal.next_mcn();
    let request = oxi_core::TransactionRequest {
        luno: terminal.luno().clone(),
        tvn: None,
        top_of_receipt: '1',
        mcn,
        track2,
        track3,
        op_code,
        amount,
        buffer_a,
        buffer_b,
        buffer_c,
        track1,
        status_data,
    };

    // Pre-validación: un bug del motor no debe mandar un TRQ inválido.
    if let Err(err) = request.to_bytes() {
        actions.push(Action::Log(format!("TRQ inválido: {err}")));
        return actions;
    }

    terminal.set_pending(Some(Pending::TransactionReply));
    terminal.set_awaiting_reply(true);
    terminal.set_pending_mcn(mcn.as_char());
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.count_attempt();
    actions.push(Action::Send(oxi_core::TerminalMessage::TransactionRequest(
        Box::new(request),
    )));
    actions.push(Action::StartTimer {
        timer: 3,
        after: terminal.config().timer_duration(3),
    });
    actions
}

/// Escribe en el buffer el valor de la FDK desde la extensión Z del estado X.
fn apply_fdk_info_value(terminal: &mut Terminal, table: &StateTableDef, fdk: Fdk) {
    let Some(extension_number) = entry_state(&table.entries, 6) else {
        return;
    };
    let Some(extension) = terminal.states().get(&extension_number).cloned() else {
        return;
    };
    if extension.state_type != 'Z' {
        return;
    }
    // Entradas 2..10 de la Z: valor por FDK en orden A,B,C,D,F,G,H,I.
    let order = [
        Fdk::A,
        Fdk::B,
        Fdk::C,
        Fdk::D,
        Fdk::F,
        Fdk::G,
        Fdk::H,
        Fdk::I,
    ];
    let Some(position) = order.iter().position(|f| *f == fdk) else {
        return;
    };
    let Some(value_text) = entry(&extension.entries, position + 2) else {
        return;
    };
    let Some(buffer_id) = entry(&table.entries, 7) else {
        return;
    };
    let zeros = buffer_id
        .chars()
        .nth(2)
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0);
    let mut value = value_text.to_owned();
    for _ in 0..zeros {
        value.push('0');
    }
    match &buffer_id[..2] {
        "01" => terminal.buffers_mut().b.push_str(&value),
        "02" => terminal.buffers_mut().c.push_str(&value),
        "03" => {
            let width = terminal.buffers().amount.len();
            terminal.buffers_mut().amount = format!("{value:0>width$}");
        }
        _ => {}
    }
}

/// Hook: tecla Cancel (usa el cancel next state del estado actual).
pub(crate) fn on_cancel(terminal: &mut Terminal) -> Vec<Action> {
    match terminal.cancel_next() {
        Some(next) => enter_state(terminal, next),
        None => vec![Action::Log(
            "Cancel sin salida configurada: ignorado".into(),
        )],
    }
}

/// A - Card Read (§2-4): pantalla + espera de tarjeta.
fn state_card_read(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    terminal.buffers_mut().reset_for_transaction();
    terminal.set_pending(Some(Pending::Card));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions
}

/// D - Pre-Set Operation Code Buffer (§2-12): aplica máscaras y transiciona.
fn state_preset_opcode(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let Some(next) = entry_state(&table.entries, 2) else {
        return default_close(terminal);
    };
    let clear = entry_mask(&table.entries, 3).unwrap_or(0);
    for bit in 0..8u8 {
        if clear & (1 << bit) != 0 {
            terminal.buffers_mut().opcode[bit as usize] = ' ';
        }
    }
    for (offset, letter) in ['A', 'B', 'C', 'D'].into_iter().enumerate() {
        let mask = entry_mask(&table.entries, 4 + offset).unwrap_or(0);
        for bit in 0..8u8 {
            if mask & (1 << bit) != 0 {
                terminal.buffers_mut().opcode[bit as usize] = letter;
            }
        }
    }
    enter_state(terminal, next)
}

/// Ancho del buffer de importe según la opción 1/008 (8 o 12 dígitos).
fn amount_width(terminal: &Terminal) -> usize {
    match terminal.config().option(1) {
        Some(code) if code & 0o10 != 0 => 12,
        _ => 8,
    }
}

/// F - Amount Entry (§2-17, Tabla 2-10).
fn state_amount_entry(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    terminal.set_pending(Some(Pending::Digits {
        target: BufferId::Amount,
        max: amount_width(terminal),
    }));
    terminal.set_fdk_next_from_entries(&table.entries, 5);
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.set_cancel_next(entry_state(&table.entries, 4));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions.push(start_timer(terminal, 0));
    actions
}

/// H - Information Entry (§2-23, Tabla 2-14).
fn state_info_entry(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let params = entry(&table.entries, 9).unwrap_or("000");
    // Tabla 2-14: 000/001 → buffer C; 002/003 → buffer B.
    let value: u8 = params.parse().unwrap_or(0);
    let target = if value <= 1 { BufferId::C } else { BufferId::B };
    terminal.set_pending(Some(Pending::Digits { target, max: 32 }));
    terminal.set_fdk_next_from_entries(&table.entries, 5);
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.set_cancel_next(entry_state(&table.entries, 4));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions.push(start_timer(terminal, 0));
    actions
}

/// J - Close (§2-30, Tabla 2-21): pantalla de recibo, tarjeta y next state.
fn state_close(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    terminal.set_pending(None);
    terminal.set_awaiting_reply(false);
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    // Tarjeta: retener o devolver según la bandera del Transaction Reply
    // (SPEC §4.3); sin reply, se devuelve.
    if terminal.card().is_some() {
        match terminal.card_return() {
            Some(oxi_core::CardReturnFlag::Retain) => {
                if let Some(screen) = entry_screen(&table.entries, 5) {
                    actions.extend(terminal.set_screen(Some(screen)));
                }
                actions.push(Action::Log("tarjeta retenida por política del host".into()));
                terminal.take_card();
            }
            _ => {
                // Tarjeta presentada: espera de retiro con timer 09 (§2-30;
                // si no la retira, se captura al vencer).
                terminal.hardware_mut().reader.state =
                    Some(crate::hardware::ReaderState::Presented);
                actions.push(Action::EjectCard);
                actions.push(start_timer(terminal, 9));
            }
        }
    }
    match entry_state(&table.entries, 3) {
        // next_state 000 = volver a esperar tarjeta (estado inicial). Solo si
        // está descargado: si no, se queda en idle (evita la cadena de cierre
        // recursiva).
        Some(next) if next.value() == 0 => {
            if terminal.states().contains_key(&next) {
                actions.extend(enter_state(terminal, next));
            } else {
                terminal.set_current(None);
            }
        }
        Some(next) => actions.extend(enter_state(terminal, next)),
        None => terminal.set_current(None),
    }
    actions
}

/// X - FDK Information Entry (§2-59).
fn state_fdk_info(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let mask = entry_mask(&table.entries, 8).unwrap_or(0);
    terminal.set_pending(Some(Pending::Keys {
        mask: FdkMask::new(mask),
    }));
    // Todas las FDK activas comparten el mismo next state (entrada 5).
    if let Some(next) = entry_state(&table.entries, 5) {
        terminal.set_fdk_next_uniform(next, FdkMask::new(mask));
    }
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.set_cancel_next(entry_state(&table.entries, 4));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions.push(start_timer(terminal, 0));
    actions
}

/// Y - Eight FDK Selection (§2-62).
fn state_eight_fdk(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let mask = entry_mask(&table.entries, 8).unwrap_or(0);
    terminal.set_pending(Some(Pending::Keys {
        mask: FdkMask::new(mask),
    }));
    if let Some(next) = entry_state(&table.entries, 5) {
        terminal.set_fdk_next_uniform(next, FdkMask::new(mask));
    }
    terminal.set_timeout_next(entry_state(&table.entries, 3));
    terminal.set_cancel_next(entry_state(&table.entries, 4));
    let mut actions = Vec::new();
    if let Some(screen) = entry_screen(&table.entries, 2) {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    actions.push(start_timer(terminal, 0));
    actions
}

/// Hook: timer del manual expirado (Time-Out State §2-89, timers de §10-17).
pub(crate) fn on_timer(terminal: &mut Terminal, timer: u8) -> Vec<Action> {
    match timer {
        // Timer 00: entrada de teclado vencida → Time-Out State.
        0 => enter_timeout_state(terminal),
        // Timer 01: la pantalla C00 no recibió respuesta → salida time-out
        // del estado original (FDK B).
        1 => {
            terminal.set_screen(None);
            match terminal.timeout_next() {
                Some(next) => enter_state(terminal, next),
                None => default_close(terminal),
            }
        }
        // Timer 03: sin Transaction Reply → salida time-out del estado I.
        3 => {
            terminal.set_awaiting_reply(false);
            terminal.set_pending(None);
            let mut actions = vec![Action::Log(
                "timer 03: sin Transaction Reply; se aplica la salida time-out del estado I".into(),
            )];
            match terminal.timeout_next() {
                Some(next) => actions.extend(enter_state(terminal, next)),
                None => actions.extend(default_close(terminal)),
            }
            actions
        }
        // Timer 07: el efectivo no fue retirado → retract (v1: log), Ready y
        // transición pendiente del reply.
        7 => {
            terminal.set_pending(None);
            terminal.set_awaiting_reply(false);
            let last = terminal
                .last_transaction()
                .map(|last| last.notes)
                .unwrap_or_default();
            let outcome = terminal.hardware_mut().dispenser.retract(last);
            let condition = crate::hardware::DeviceCondition {
                device: 'E',
                status: crate::hardware::Dispenser::status_field(outcome, [0; 4]),
                severity: '2',
                diagnostic: String::new(),
                supplies: Some(terminal.hardware().dispenser.notes_in_cassettes()),
            };
            let mut actions = vec![Action::Log(
                "timer 07: efectivo retraído a la caja de retract".into(),
            )];
            actions.extend(terminal.set_screen(None));
            actions.push(Action::Send(terminal.unsolicited(condition)));
            actions.push(send_ready_after_reply(terminal));
            if let Some(next) = terminal.take_post_cash_next() {
                actions.extend(enter_state(terminal, next));
            }
            actions
        }
        // Timer 09: la tarjeta presentada no fue retirada → captura (§2-30).
        9 => {
            terminal.hardware_mut().reader.captures += 1;
            terminal.hardware_mut().reader.state = Some(crate::hardware::ReaderState::Captured);
            terminal.take_card();
            let condition = crate::hardware::DeviceCondition {
                device: 'D',
                status: "1".into(),
                severity: '2',
                diagnostic: String::new(),
                supplies: None,
            };
            vec![
                Action::Log("timer 09: tarjeta capturada por no ser retirada".into()),
                Action::Send(terminal.unsolicited(condition)),
            ]
        }
        other => vec![Action::Log(format!(
            "timer {other:02} sin manejador (fases posteriores)"
        ))],
    }
}

/// Inicia un timer con su duración configurada (Action).
pub(crate) fn start_timer(terminal: &Terminal, timer: u8) -> Action {
    Action::StartTimer {
        timer,
        after: terminal.config().timer_duration(timer),
    }
}

/// Hook: el cardholder retiró el efectivo (cierra la espera del timer 07).
pub(crate) fn on_cash_taken(terminal: &mut Terminal) -> Vec<Action> {
    if terminal.pending() != Some(&Pending::Cash) {
        return Vec::new();
    }
    terminal.set_pending(None);
    terminal.set_awaiting_reply(false);
    let mut actions = vec![
        Action::CancelTimer(7),
        Action::Log("efectivo retirado".into()),
        send_ready_after_reply(terminal),
    ];
    if let Some(next) = terminal.take_post_cash_next() {
        actions.extend(enter_state(terminal, next));
    }
    actions
}

/// Time-Out State fijo (§2-89): pantalla C00, timer 01, FDK A reintenta y
/// FDK B sale por el time-out next state del estado original.
fn enter_timeout_state(terminal: &mut Terminal) -> Vec<Action> {
    terminal.set_retry_state(terminal.current_state());
    let mut actions = terminal.set_screen(Some(ScreenSelector::Group {
        letter: 'C',
        digits: 0,
    }));
    actions.push(Action::StartTimer {
        timer: 1,
        after: terminal.config().timer_duration(1),
    });
    // FDK A = reintentar el estado; FDK B = salir por time-out (§2-89).
    terminal.set_fdk_next(oxi_core::Fdk::A, terminal.retry_state());
    terminal.set_fdk_next(oxi_core::Fdk::B, terminal.timeout_next());
    actions.push(Action::Log("time-out state: pantalla C00".into()));
    actions
}

/// Ready tras un Transaction Reply (descriptor 'B' si opción 1/001).
pub(crate) fn send_ready_after_reply(terminal: &Terminal) -> Action {
    Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
        luno: terminal.luno().clone(),
        descriptor: if terminal.config().option(1) == Some(1) {
            oxi_core::StatusDescriptor::ReadyAlternate
        } else {
            oxi_core::StatusDescriptor::Ready
        },
        data: Vec::new(),
    })
}

/// W - FDK Switch (§2-58): conmuta según el keycode guardado en el buffer.
fn state_fdk_switch(terminal: &mut Terminal, table: &StateTableDef) -> Vec<Action> {
    let Some(keycode) = terminal.buffers().fdk else {
        return vec![Action::Log(
            "estado W sin keycode en el buffer de FDK".into(),
        )]
        .into_iter()
        .chain(default_close(terminal))
        .collect();
    };
    // Entradas 2..10: next por FDK en orden A,B,C,D,F,G,H,I.
    let order = [
        Fdk::A,
        Fdk::B,
        Fdk::C,
        Fdk::D,
        Fdk::F,
        Fdk::G,
        Fdk::H,
        Fdk::I,
    ];
    let Some(position) = order.iter().position(|f| f.as_char() == keycode) else {
        return default_close(terminal);
    };
    match entry_state(&table.entries, position + 2) {
        Some(next) => enter_state(terminal, next),
        None => default_close(terminal),
    }
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
                    // pantalla 010, good 002, error 001, rc1 002, rc2 002,
                    // rc3 002, card_return 000, no_fit 001
                    entries: "010002001002002002000001".into(),
                },
                StateTableDef {
                    number: StateNumber::new(2).expect("en rango"),
                    state_type: 'D',
                    // next 010, sin máscaras
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

#[cfg(test)]
mod interpreter_tests {
    use oxi_core::{CustomisationData, Fdk, ScreenSelector, StateNumber, StateTableDef};

    use crate::event::{Action, Event, Key};
    use crate::terminal::{apply_customisation, Terminal};

    fn terminal_with(states: Vec<StateTableDef>) -> Terminal {
        let mut terminal = Terminal::new();
        let _ = apply_customisation(&mut terminal, CustomisationData::StateTables(states));
        terminal
    }

    fn table(number: u16, state_type: char, entries: &str) -> StateTableDef {
        StateTableDef {
            number: StateNumber::new(number).expect("número en rango"),
            state_type,
            entries: entries.into(),
        }
    }

    fn enter(terminal: &mut Terminal, state: u16) {
        let number = StateNumber::new(state).expect("número en rango");
        let _ = crate::state_exec::enter_state(terminal, number);
    }

    fn displays_screen(actions: &[Action], screen: u16) -> bool {
        actions.iter().any(|action| {
            matches!(
                action,
                Action::Display(Some(ScreenSelector::Customer(id))) if id.value() == screen
            )
        })
    }

    /// A: muestra pantalla y espera tarjeta.
    #[test]
    fn state_a_should_display_screen_and_wait_for_card() {
        let mut terminal = terminal_with(vec![table(0, 'A', "010001002002002002000001")]);
        enter(&mut terminal, 0);
        assert!(matches!(
            terminal.screen(),
            Some(ScreenSelector::Customer(id)) if id.value() == 10
        ));
    }

    /// A: con pista 2 presente y condición 002, va al good read next state
    /// (aquí un J que cierra y vuelve a 000, el estado A).
    #[test]
    fn state_a_should_go_to_good_read_on_track2() {
        let mut terminal = terminal_with(vec![
            table(0, 'A', "010001002002002002000001"),
            table(1, 'J', "052000052052052052052000"),
        ]);
        enter(&mut terminal, 0);
        let actions = terminal.handle(Event::CardInserted(crate::event::Card {
            track2: Some(";1234=99?".into()),
            ..crate::event::Card::default()
        }));
        assert!(displays_screen(&actions, 52), "el J mostró su pantalla");
        assert_eq!(
            terminal.current_state().map(|state| state.value()),
            Some(0),
            "vuelve a esperar tarjeta"
        );
    }

    /// D: las máscaras escriben el op-code y transicionan (§2-12).
    /// El J de cierre usa next 255 (idle) para no ciclar.
    #[test]
    fn state_d_should_apply_opcode_masks() {
        let mut terminal = terminal_with(vec![
            table(0, 'D', "001255008000000000000000"),
            table(1, 'J', "052255052052052052052000"),
        ]);
        enter(&mut terminal, 0);
        assert_eq!(terminal.buffers().opcode[3], 'A');
        assert_eq!(terminal.buffers().opcode[0], ' ');
        assert_eq!(terminal.current_state(), None, "J con next 255 queda idle");
    }

    /// F: captura de importe con shift a la derecha y Enter = FDK A (§2-17).
    #[test]
    fn state_f_should_capture_amount_and_enter_goes_fdk_a() {
        let mut terminal = terminal_with(vec![
            table(0, 'F', "020200200001255255255020"),
            table(1, 'J', "052255052052052052052000"),
        ]);
        enter(&mut terminal, 0);
        for digit in ['1', '0', '0'] {
            terminal.handle(Event::KeyPressed(Key::Digit(digit)));
        }
        assert_eq!(terminal.buffers().amount, "00000100");
        let actions = terminal.handle(Event::KeyPressed(Key::Enter));
        assert!(
            displays_screen(&actions, 52),
            "Enter = FDK A → estado 001 (J)"
        );
    }

    /// Clear borra el importe a ceros (§2-17).
    #[test]
    fn state_f_clear_should_reset_amount_to_zeros() {
        let mut terminal = terminal_with(vec![table(0, 'F', "020200200001255255255020")]);
        enter(&mut terminal, 0);
        terminal.handle(Event::KeyPressed(Key::Digit('5')));
        terminal.handle(Event::KeyPressed(Key::Clear));
        assert_eq!(terminal.buffers().amount, "00000000");
    }

    /// H: parámetro 002 → buffer B con eco 'X' (§2-23, Tabla 2-14).
    #[test]
    fn state_h_should_store_in_buffer_b_per_params_002() {
        let mut terminal = terminal_with(vec![
            table(0, 'H', "030200200001255255255002"),
            table(1, 'J', "052255052052052052052000"),
        ]);
        enter(&mut terminal, 0);
        for digit in ['7', '7', '7'] {
            terminal.handle(Event::KeyPressed(Key::Digit(digit)));
        }
        assert_eq!(terminal.buffers().b, "777");
        let actions = terminal.handle(Event::KeyPressed(Key::Enter));
        assert!(displays_screen(&actions, 52));
    }

    /// H: parámetro 001 → buffer C (Tabla 2-14).
    #[test]
    fn state_h_should_store_in_buffer_c_per_params_001() {
        let mut terminal = terminal_with(vec![table(0, 'H', "030200200001255255255001")]);
        enter(&mut terminal, 0);
        terminal.handle(Event::KeyPressed(Key::Digit('4')));
        assert_eq!(terminal.buffers().c, "4");
        assert!(terminal.buffers().b.is_empty());
    }

    /// Y: la FDK guarda keycode y escribe el op-code en las posiciones de la
    /// máscara (§2-62): buffer_positions 008 → bit 3; W conmuta por keycode.
    #[test]
    fn state_y_should_set_keycode_and_opcode_positions() {
        let mut terminal = terminal_with(vec![
            table(0, 'Y', "070200201004255008003000"),
            table(4, 'W', "005006000000000000000000"),
            table(5, 'J', "052255052052052052052000"),
            table(6, 'J', "053255053053053053053000"),
        ]);
        enter(&mut terminal, 0);
        let actions = terminal.handle(Event::KeyPressed(Key::Fdk(Fdk::B)));
        assert_eq!(terminal.buffers().fdk, Some('B'));
        assert_eq!(terminal.buffers().opcode[3], 'B');
        // W con keycode B → estado 006 → J que muestra la pantalla 053.
        assert!(displays_screen(&actions, 53), "B debe ir por el J de 006");
    }

    /// X: el valor de la FDK se toma de la extensión Z y se escribe en el
    /// buffer con los ceros del buffer_id (§2-61): 033 = importe + 3 ceros.
    #[test]
    fn state_x_should_write_extension_value_to_amount_buffer() {
        let mut terminal = terminal_with(vec![
            table(25, 'X', "020200201027026033255"),
            table(26, 'Z', "010020030040050060080100"),
            table(27, 'J', "052255052052052052052000"),
        ]);
        enter(&mut terminal, 25);
        let actions = terminal.handle(Event::KeyPressed(Key::Fdk(Fdk::C)));
        // Valor de C = 030 + 3 ceros = "030000" → importe de 8: "00030000".
        assert_eq!(terminal.buffers().amount, "00030000");
        assert!(displays_screen(&actions, 52), "transiciona al J de 027");
    }

    /// Cancel usa el cancel next state del estado corriente.
    #[test]
    fn cancel_should_follow_cancel_next_state() {
        let mut terminal = terminal_with(vec![
            // screen 020, timeout 200, cancel 001, FDK A 001, resto 255
            table(0, 'F', "020200001001255255255020"),
            table(1, 'J', "052255052052052052052000"),
        ]);
        enter(&mut terminal, 0);
        let actions = terminal.handle(Event::KeyPressed(Key::Cancel));
        assert!(displays_screen(&actions, 52));
    }

    /// Un estado inexistente dispara el default close (estado 200 si está).
    #[test]
    fn missing_state_should_run_default_close() {
        let mut terminal = terminal_with(vec![table(200, 'J', "052255052052052052052000")]);
        let actions =
            crate::state_exec::enter_state(&mut terminal, StateNumber::new(77).expect("en rango"));
        assert!(displays_screen(&actions, 52));
    }

    /// Los buffers generales no hacen shift: solo el importe.
    #[test]
    fn general_buffers_should_append_without_shift() {
        let mut terminal = terminal_with(vec![table(0, 'H', "030200200001255255255003")]);
        enter(&mut terminal, 0);
        for digit in ['0', '1', '2'] {
            terminal.handle(Event::KeyPressed(Key::Digit(digit)));
        }
        assert_eq!(terminal.buffers().b, "012");
    }
}
