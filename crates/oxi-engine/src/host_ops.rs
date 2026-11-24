//! Manejo de mensajes del host: matriz de modos (SPEC §4.5) + aplicación.
//!
//! La política vive en `oxi_core::disposition` (Tablas 10-38/39/40); aquí se
//! ejecuta: responder, aplicar la customización o rechazar con log.

use std::time::Duration;

use oxi_core::{
    Disposition, HostMessage, InteractiveTransactionResponse, ModeContext, OperationalMode,
    RawFields, StateNumber, StatusDescriptor, TerminalCommand, TerminalMessage,
};

use crate::crypto::{CryptoError, KeyMode};
use crate::event::{Action, Pending};
use crate::state_exec::enter_state;
use crate::terminal::{apply_customisation, ready_action, send, Terminal, ITR_TIMER};

/// Procesa un mensaje del host según el modo operativo.
pub(crate) fn handle_host(terminal: &mut Terminal, message: HostMessage) -> Vec<Action> {
    let context = ModeContext {
        mode: terminal.mode(),
        in_transaction_request: terminal.awaiting_reply(),
    };
    match oxi_core::disposition(&message, &context) {
        Disposition::NoAction => Vec::new(),
        Disposition::Reject => {
            if let Some(code) = mode_reject_code(terminal.mode(), &message) {
                vec![
                    send(terminal.specific_command_reject(code)),
                    Action::Log(format!(
                        "mensaje rechazado por el modo operativo: {code} (§10-37)"
                    )),
                ]
            } else {
                reject(terminal, &message)
            }
        }
        Disposition::RespondConfigId => vec![send(
            terminal.terminal_state('6', vec![config_id_text(terminal)]),
        )],
        Disposition::RespondRequestedInfo => respond_info(terminal, &message),
        Disposition::AcceptReady
        | Disposition::ChangeToInService
        | Disposition::ChangeToOutOfServiceIdle => accept(terminal, message),
        Disposition::HoldUntilSupplyExitLastWins | Disposition::HoldUntilOutOfService => {
            vec![Action::Log(
                "comando retenido por el modo (cola de secuencia: fase 6)".into(),
            )]
        }
        Disposition::AcceptOnlyIfNoTransaction => {
            // Cambios de clave: se aceptan solo sin transacción en curso
            // (Tabla 10-38 'A3'); se aplican si son del tipo soportado.
            if terminal.awaiting_reply() {
                vec![
                    send(terminal.specific_command_reject("C15")),
                    Action::Log("cambio de claves durante una transacción: C/15".into()),
                ]
            } else {
                accept(terminal, message)
            }
        }
        Disposition::AcceptIfTranReqElseReject => {
            if terminal.awaiting_reply() {
                match message {
                    HostMessage::InteractiveTransactionResponse(itr) => accept_itr(terminal, itr),
                    other => reject(terminal, &other),
                }
            } else {
                vec![
                    send(terminal.specific_command_reject("C04")),
                    Action::Log("ITR fuera del estado I: C/04 (§10-37)".into()),
                ]
            }
        }
    }
}

/// Responde un frame que no se pudo parsear con el reject de Tabla 10-37.
pub(crate) fn handle_parse_error(terminal: &Terminal, reject: &str) -> Vec<Action> {
    if terminal.mode() == OperationalMode::PowerUp {
        return vec![Action::Log(format!(
            "frame inválido en Power-Up (sin respuesta): {reject}"
        ))];
    }
    let response = terminal.specific_command_reject(reject);
    let sent = match &response {
        TerminalMessage::SolicitedStatus {
            descriptor: StatusDescriptor::SpecificCommandReject,
            ..
        } => reject,
        _ => "A",
    };
    vec![
        send(response),
        Action::Log(format!("frame inválido: reject {sent} (Tabla 10-37)")),
    ]
}

/// Código C específico para mensajes válidos recibidos en el modo incorrecto.
fn mode_reject_code(mode: OperationalMode, message: &HostMessage) -> Option<&'static str> {
    if let HostMessage::Unknown { raw } = message {
        return Some(unknown_reject_code(raw));
    }
    match (mode, message) {
        // ITR en OOS/Supply; en In-Service el caso "no estado I" se maneja en
        // AcceptIfTranReqElseReject (C04).
        (
            OperationalMode::OutOfService | OperationalMode::Supply,
            HostMessage::InteractiveTransactionResponse(_),
        ) => Some("C03"),
        (
            OperationalMode::OutOfService | OperationalMode::Supply,
            HostMessage::TransactionReply(_),
        ) => Some("C03"),
        // Reply fuera del estado de transacción estando In-Service.
        (OperationalMode::InService, HostMessage::TransactionReply(_)) => Some("C01"),
        // Tabla 10-37 C04: comandos 4, 5, 6, 8 no aceptados In-Service.
        (
            OperationalMode::InService,
            HostMessage::TerminalCommand(
                TerminalCommand::SendSupplyCounters
                | TerminalCommand::SendTallyInformation { .. }
                | TerminalCommand::SendErrorLog
                | TerminalCommand::SendDateAndTime,
            ),
        ) => Some("C04"),
        _ => None,
    }
}

/// Acepta una Interactive Transaction Response: pantalla dinámica, teclas
/// activas y screen timer (manual §10-79).
fn accept_itr(terminal: &mut Terminal, itr: InteractiveTransactionResponse) -> Vec<Action> {
    let screen_data = itr.screen_data.clone();
    let display_flag = itr.display_flag;
    let active_keys = itr.active_keys.clone();
    let after = itr
        .screen_timer_ticks()
        .map(|ticks| Duration::from_millis(u64::from(ticks) * 800));

    terminal.buffers_mut().b.clear();
    terminal.set_pending(Some(Pending::Itr { response: itr }));
    let mut actions = vec![
        // La respuesta final llegará como un nuevo TRQ; el timer 03 original
        // se suspende mientras el cardholder interactúa.
        Action::CancelTimer(3),
        Action::CancelTimer(ITR_TIMER),
        Action::Log(format!(
            "ITR: pantalla dinámica (display '{display_flag}', teclas '{active_keys}')"
        )),
    ];
    actions.extend(terminal.set_screen(None));
    actions.push(Action::DisplayData(screen_data));
    actions.push(Action::StartTimer {
        timer: ITR_TIMER,
        after,
    });
    actions
}

/// Acepta y aplica el mensaje (respondiendo Ready).
fn accept(terminal: &mut Terminal, message: HostMessage) -> Vec<Action> {
    let mut actions = Vec::new();
    match message {
        HostMessage::TerminalCommand(command) => match command {
            TerminalCommand::GoInService => {
                actions.extend(terminal.set_mode(oxi_core::OperationalMode::InService));
                terminal.set_awaiting_reply(false);
                terminal.set_pending(None);
                // Los buffers de transacción arrancan limpios (manual §2-4:
                // los inicializa el estado A; lo hacemos ya al entrar en
                // servicio para no arrastrar la transacción anterior).
                terminal.buffers_mut().reset_for_transaction();
                // Estado inicial estándar (manual §2-4: A como estado 000).
                let initial = StateNumber::new(0).expect("000 es válido");
                if terminal.states().contains_key(&initial) {
                    actions.push(ready_action(terminal, false));
                    actions.extend(enter_state(terminal, initial));
                } else {
                    actions.push(ready_action(terminal, false));
                    actions.push(Action::Log(
                        "Go In-Service sin estado 000 descargado: esperando customización".into(),
                    ));
                }
            }
            TerminalCommand::GoOutOfService { .. } => {
                actions.extend(terminal.set_mode(oxi_core::OperationalMode::OutOfService));
                terminal.set_pending(None);
                terminal.set_awaiting_reply(false);
                terminal.set_current(None);
                actions.push(ready_action(terminal, false));
            }
            // Los comandos informativos se manejan en RespondRequestedInfo.
            other => {
                actions.push(ready_action(terminal, false));
                actions.push(Action::Log(format!("comando aceptado: {other:?}")));
            }
        },
        HostMessage::Customisation(data) => {
            actions.extend(apply_customisation(terminal, data));
            actions.push(ready_action(terminal, false));
        }
        HostMessage::TransactionReply(reply) => {
            actions.extend(execute_transaction_reply(terminal, reply));
        }
        HostMessage::InteractiveTransactionResponse(_) => {
            actions.push(send(terminal.command_reject()));
            actions.push(Action::Log("ITR: fase 8".into()));
        }
        HostMessage::EncryptionKeyChange { .. } => {
            actions.push(send(terminal.command_reject()));
            actions.push(Action::Log(
                "Encryption Key Change (sub-clase 3, sin EPP): reject".into(),
            ));
        }
        HostMessage::ExtendedEncryptionKeyChange { raw } => {
            actions.extend(apply_extended_key_change(terminal, &raw));
        }
        HostMessage::Unknown { raw } => {
            let code = unknown_reject_code(&raw);
            actions.push(send(terminal.specific_command_reject(code)));
            actions.push(Action::Log(format!(
                "mensaje no modelado: reject {code} (Tabla 10-37)"
            )));
        }
    }
    actions
}

/// Respuesta a comandos que piden información (Tabla 10-40, 'D').
fn respond_info(terminal: &Terminal, message: &HostMessage) -> Vec<Action> {
    let HostMessage::TerminalCommand(command) = message else {
        return vec![send(terminal.command_reject())];
    };
    match command {
        TerminalCommand::SendConfigurationInformation { .. } => {
            // g1 '1' = información de configuración (§9-19). Los campos
            // detallados (hardware, supplies, fitness) llegan con la fase 4.
            vec![send(terminal.terminal_state('1', vec![]))]
        }
        TerminalCommand::SendSupplyCounters => {
            // Tabla 9-8: 'F' + g1 '2' + TSN(4) + acumuladas(7) + 4×20
            // contadores + capturadas(5) + sobres(5) + cámara(5) + serial(5).
            vec![send(terminal.terminal_state(
                '2',
                vec![terminal.supply_counters_payload()],
            ))]
        }
        TerminalCommand::SendDateAndTime => {
            // g1 '5' = fecha y hora del terminal (§9-19); datos en fase 4.
            vec![send(terminal.terminal_state('5', vec![]))]
        }
        TerminalCommand::SendTallyInformation { .. } | TerminalCommand::SendErrorLog => {
            // No soportados en Advance NDC (manual §10-4, §9-35/§9-36).
            vec![
                send(terminal.specific_command_reject("E01")),
                Action::Log("tallies / error log no soportados por Advance NDC (E/01)".into()),
            ]
        }
        _ => vec![send(terminal.command_reject())],
    }
}

/// Ejecuta el Transaction Reply (manual §10-58): valida MCN, imprime,
/// dispensa, muestra pantalla y transiciona; Ready se difiere al retirar el
/// efectivo cuando hay dispensado.
fn execute_transaction_reply(
    terminal: &mut Terminal,
    reply: oxi_core::TransactionReply,
) -> Vec<Action> {
    let mut actions = Vec::new();
    // MCN: '0' anula la verificación (manual §9-2). Si no coincide:
    // specific reject B09 si la opción 12 está activa (§10-88), si no 'A'.
    if let Some(mcn) = reply.mcn {
        let expected = terminal.pending_mcn();
        if mcn != '0' && Some(mcn) != expected {
            actions.push(Action::Send(terminal.specific_command_reject("B09")));
            actions.push(Action::Log(format!(
                "MCN del reply '{mcn}' no coincide con el TRQ {expected:?}: reject"
            )));
            return actions;
        }
    }
    terminal.set_awaiting_reply(false);
    terminal.cancel_pending_mcn();
    terminal.set_card_return(reply.card_return);

    // Grupos de impresión (flag 1 journal, 2 receipt, 3 ambos; §10-58).
    // Las fallas de impresora generan unsolicited status y la transacción
    // continúa (§9-58: no requieren recuperación de Central).
    for group in reply.print_groups {
        let mut fault = None;
        match group.flag {
            oxi_core::PrinterFlag::Journal => {
                if terminal.hardware_mut().journal.print(&group.data).is_err() {
                    fault = Some(('H', terminal.hardware().journal.supplies_status()));
                }
            }
            oxi_core::PrinterFlag::Receipt => {
                if terminal.hardware_mut().receipt.print(&group.data).is_err() {
                    fault = Some(('G', terminal.hardware().receipt.supplies_status()));
                }
            }
            oxi_core::PrinterFlag::JournalAndReceipt => {
                let journal_ok = terminal.hardware_mut().journal.print(&group.data).is_ok();
                let receipt_ok = terminal.hardware_mut().receipt.print(&group.data).is_ok();
                if !journal_ok {
                    fault = Some(('H', terminal.hardware().journal.supplies_status()));
                } else if !receipt_ok {
                    fault = Some(('G', terminal.hardware().receipt.supplies_status()));
                }
            }
            oxi_core::PrinterFlag::NoPrint => {}
        }
        if let Some((device, supplies)) = fault {
            let condition = crate::hardware::DeviceCondition {
                device,
                status: "1".into(),
                severity: '4',
                diagnostic: String::new(),
                supplies: Some(supplies),
            };
            actions.push(Action::Send(terminal.unsolicited(condition)));
            actions.push(Action::Log(format!(
                "falla de impresora '{device}' al imprimir; la transacción continúa"
            )));
        }
        actions.push(Action::Print {
            printer: group.flag,
            data: group.data,
        });
    }

    // Dispensado: el hardware virtual valida casetes y fallas inyectadas
    // (Tabla 9-31). Ready y transición se difieren al retiro (timer 07).
    let mut ready_deferred = false;
    if let Some(notes) = reply.notes {
        let counts = notes.counts();
        let (outcome, actual) = terminal.hardware_mut().dispenser.dispense(counts);
        terminal.set_last_transaction(crate::terminal::LastTransaction {
            serial: reply
                .serial
                .as_deref()
                .and_then(|serial| serial.parse().ok())
                .unwrap_or(0),
            status: '1',
            notes: actual,
        });
        let delivered: u16 = actual.iter().sum();
        match outcome {
            crate::hardware::DispenseOutcome::Complete => {
                terminal.set_pending(Some(crate::event::Pending::Cash));
                actions.push(Action::Dispense(notes));
                actions.push(crate::state_exec::start_timer(terminal, 7));
                ready_deferred = true;
            }
            crate::hardware::DispenseOutcome::Short => {
                // Dispensado parcial: se entrega lo disponible y se reporta
                // falla de dispositivo ('8') con los conteos reales.
                let actual_notes = oxi_core::NoteCounts::new(actual)
                    .unwrap_or_else(|_| oxi_core::NoteCounts::default());
                let condition = crate::hardware::DeviceCondition {
                    device: 'E',
                    status: crate::hardware::Dispenser::status_field(outcome, actual),
                    severity: '2',
                    diagnostic: String::new(),
                    supplies: Some(terminal.hardware().dispenser.notes_in_cassettes()),
                };
                actions.push(Action::Send(terminal.device_fault(condition)));
                if delivered > 0 {
                    terminal.set_pending(Some(crate::event::Pending::Cash));
                    actions.push(Action::Dispense(actual_notes));
                    actions.push(crate::state_exec::start_timer(terminal, 7));
                    ready_deferred = true;
                }
            }
            crate::hardware::DispenseOutcome::None
            | crate::hardware::DispenseOutcome::Retracted => {
                // Sin dispensado: falla de dispositivo; el flujo sigue al
                // next state del reply (§10-58: respuesta = device fault).
                let condition = crate::hardware::DeviceCondition {
                    device: 'E',
                    status: crate::hardware::Dispenser::status_field(outcome, actual),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: Some(terminal.hardware().dispenser.notes_in_cassettes()),
                };
                actions.push(Action::Send(terminal.device_fault(condition)));
            }
        }
    }

    if let Some(screen) = reply.screen {
        actions.extend(terminal.set_screen(Some(screen)));
    }
    if let Some(update) = &reply.screen_display_update {
        actions.push(Action::Log(format!(
            "actualización de pantalla del reply ignorada en v1: {update}"
        )));
    }

    // Fin de la transacción (§10-58/§10-74): con dispensado pendiente, la
    // transición y el Ready esperan al retiro del efectivo (timer 07).
    actions.push(Action::CancelTimer(3));
    if ready_deferred {
        terminal.set_post_cash_next(Some(reply.next_state));
        if let Some(function_id) = reply.function_id {
            actions.push(Action::Log(format!(
                "Transaction Reply: function '{function_id}' con dispensado; \
                 next state {} diferido al retiro",
                reply.next_state
            )));
        }
    } else {
        actions.push(Action::Send(terminal.ready(true)));
        if let Some(function_id) = reply.function_id {
            actions.push(Action::Log(format!(
                "Transaction Reply: function '{function_id}' → estado {}",
                reply.next_state
            )));
        }
        actions.extend(crate::state_exec::enter_state(terminal, reply.next_state));
    }
    actions
}

/// Ejecuta el Extended Encryption Key Change (Tabla 10-12): modificador +
/// tamaño en hex + datos decimales. v1 soporta claves simples (§10-45:
/// tamaño `018` = 24 chars = 8 bytes); doble longitud queda documentado.
fn apply_extended_key_change(terminal: &mut Terminal, raw: &oxi_core::RawFields) -> Vec<Action> {
    let mut actions = Vec::new();
    let Ok(head) = raw.ascii_field(3) else {
        return vec![send(terminal.command_reject())];
    };
    let modifier = head.chars().nth(1).unwrap_or('?');
    let size = raw.ascii_field(4).unwrap_or_default();
    let data = raw.ascii_field(5).unwrap_or_default();
    match modifier {
        // '2': nueva clave de comunicaciones, descifrada con la master.
        '2' => {
            if size != "018" {
                actions.push(send(terminal.command_reject()));
                actions.push(Action::Log(format!(
                    "EKC: tamaño '{size}' no soportado en v1 (solo 018, single)"
                )));
                return actions;
            }
            let Some(master) = terminal.security().master_key_bytes() else {
                actions.push(send(terminal.command_reject()));
                actions.push(Action::Log("EKC sin clave master configurada".into()));
                return actions;
            };
            let result = master.and_then(|master| {
                let encrypted = crate::crypto::decode_decimal_key(data)?;
                let block: [u8; 8] = encrypted
                    .try_into()
                    .map_err(|_| CryptoError::InvalidKey("la clave debe ser de 8 bytes".into()))?;
                let decrypted = crate::crypto::decrypt_block(&master, block, KeyMode::Single)?;
                Ok(decrypted)
            });
            match result {
                Ok(new_key) => {
                    let hex: String = new_key.iter().map(|byte| format!("{byte:02X}")).collect();
                    terminal.security_mut().pin_key = Some(hex);
                    terminal.security_mut().pin_key_mode = KeyMode::Single;
                    actions.push(send(terminal.ready(false)));
                    actions.push(Action::Log(format!(
                        "EKC: nueva clave de comunicaciones cargada (KVV {})",
                        terminal
                            .pin_kvv()
                            .and_then(|kvv| kvv.ok())
                            .unwrap_or_else(|| "??????".into())
                    )));
                }
                Err(err) => {
                    actions.push(send(terminal.command_reject()));
                    actions.push(Action::Log(format!("EKC rechazado: {err}")));
                }
            }
        }
        // '4': usar la clave local 'B' como clave de comunicaciones.
        '4' => match terminal.security().local_b_key_bytes() {
            Some(Ok(key)) => {
                let hex: String = key.iter().map(|byte| format!("{byte:02X}")).collect();
                terminal.security_mut().pin_key = Some(hex);
                terminal.security_mut().pin_key_mode = KeyMode::Single;
                actions.push(send(terminal.ready(false)));
                actions.push(Action::Log("EKC: clave local B activada".into()));
            }
            _ => {
                actions.push(send(terminal.command_reject()));
                actions.push(Action::Log("EKC: sin clave local B configurada".into()));
            }
        },
        other => {
            actions.push(send(terminal.command_reject()));
            actions.push(Action::Log(format!(
                "EKC modificador '{other}' no soportado en v1 (EPP/EID en fase posterior)"
            )));
        }
    }
    actions
}

/// Config ID como 4 dígitos (0000 = instalación, manual §10-33).
fn config_id_text(terminal: &Terminal) -> String {
    format!("{:04}", terminal.config_id().unwrap_or(0))
}

/// Código B para mensajes que el parser no reconoce (Tabla 10-37).
fn unknown_reject_code(raw: &RawFields) -> &'static str {
    let class = raw.ascii_field(0).ok().and_then(|head| head.chars().next());
    match class {
        Some('1') => "B04", // código de comando de terminal ilegal
        Some('3') => "B02", // sub-clase/identificador de load ilegal
        _ => "B01",         // clase ilegal
    }
}

/// Rechazo de comando con log del motivo.
fn reject(terminal: &Terminal, message: &HostMessage) -> Vec<Action> {
    vec![
        send(terminal.command_reject()),
        Action::Log(format!(
            "mensaje rechazado por el modo operativo: {message:?}"
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxi_core::{OperationalMode, ParseOptions, RawFields, ScreenId, ScreenSelector};

    use crate::event::Event;
    use crate::state_exec::tests_support::*;
    use crate::terminal::Terminal;

    fn parse_line(line: &[u8]) -> HostMessage {
        let raw = RawFields::parse(line);
        oxi_core::parse_host(&raw, &ParseOptions::default()).expect("mensaje sintético válido")
    }

    #[test]
    fn screens_load_in_out_of_service_should_be_ready() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        let message = parse_line(b"3\x1c\x1c\x1c11\x1c000HOLA");
        let actions = terminal.handle(Event::HostMessage(message));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::Ready,
                ..
            })
        )));

        let screen = terminal.screens().get(&ScreenSelector::Customer(
            ScreenId::new(0).expect("en rango"),
        ));
        assert!(screen.is_some(), "la pantalla 000 quedó cargada");
    }

    /// Tabla 10-38: la customización en In-Service se rechaza (§10-87).
    #[test]
    fn screens_load_in_service_should_be_rejected() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        let message = parse_line(b"3\x1c\x1c\x1c11\x1c000HOLA");
        let actions = terminal.handle(Event::HostMessage(message));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::CommandReject,
                ..
            })
        )));
        assert!(terminal.screens().is_empty(), "no se aplicó la pantalla");
    }

    #[test]
    fn config_id_request_should_respond_terminal_state() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        let message = parse_line(b"1\x1c\x1c\x1c3");
        let actions = terminal.handle(Event::HostMessage(message));
        let sent = actions.iter().find_map(|a| match a {
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::TerminalState,
                data,
                ..
            }) => Some(data.clone()),
            _ => None,
        });
        let data = sent.expect("debe responder Terminal State");
        assert_eq!(data, vec!["60000".to_string()], "g1 '6' + config id 0000");
    }

    #[test]
    fn go_in_service_should_enter_state_000_when_loaded() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        load_minimal_customisation(&mut terminal);
        let message = parse_line(b"1\x1c\x1c\x1c1");
        let actions = terminal.handle(Event::HostMessage(message));
        assert_eq!(terminal.mode(), OperationalMode::InService);
        assert_eq!(terminal.current_state().map(|s| s.value()), Some(0));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::Ready,
                ..
            })
        )));
    }

    #[test]
    fn go_out_of_service_should_clear_state_and_answer_ready() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        let message = parse_line(b"1\x1c\x1c\x1c2");
        let actions = terminal.handle(Event::HostMessage(message));
        assert_eq!(terminal.mode(), OperationalMode::OutOfService);
        assert_eq!(terminal.current_state(), None);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::Ready,
                ..
            })
        )));
    }

    #[test]
    fn ecpl_should_set_luno_and_options() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        let message = parse_line(b"3\x1c\x1c\x1c1A\x1c002\x1c01001");
        terminal.handle(Event::HostMessage(message));
        assert_eq!(terminal.luno().as_str(), "002");
    }

    #[test]
    fn unknown_message_should_be_rejected_everywhere() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        let message = HostMessage::Unknown {
            raw: RawFields::parse(b"9\x1c\x1c\x1c1"),
        };
        let actions = terminal.handle(Event::HostMessage(message));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::CommandReject,
                ..
            })
        )));
    }

    #[test]
    fn customisation_after_power_on_should_be_accepted_in_out_of_service() {
        let mut terminal = Terminal::new();
        terminal.handle(Event::PoweredOn);
        let message = parse_line(b"3\x1c\x1c\x1c11\x1c000HOLA");
        let actions = terminal.handle(Event::HostMessage(message));
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                    descriptor: oxi_core::StatusDescriptor::Ready,
                    ..
                })
            )),
            "tras el power-up el terminal queda fuera de servicio y acepta la descarga"
        );
    }

    #[test]
    fn malformed_frame_should_send_specific_reject_with_option_12() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        terminal.config_mut().options.insert(12, 1);
        let actions = handle_parse_error(&terminal, "B08");
        let Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
            descriptor: oxi_core::StatusDescriptor::SpecificCommandReject,
            data,
            ..
        }) = &actions[0]
        else {
            panic!("se esperaba Specific Command Reject: {actions:?}");
        };
        assert_eq!(data, &vec!["B08".to_string()]);
    }

    #[test]
    fn malformed_frame_should_fall_back_to_command_reject_without_option_12() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        let actions = handle_parse_error(&terminal, "B08");
        assert!(matches!(
            &actions[0],
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::CommandReject,
                ..
            })
        ));
    }

    #[test]
    fn unknown_terminal_command_should_specific_reject_b04() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        terminal.config_mut().options.insert(12, 1);
        let message = parse_line(b"1\x1c\x1c\x1cZ");
        assert!(matches!(message, HostMessage::Unknown { .. }));
        let actions = terminal.handle(Event::HostMessage(message));
        let Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
            descriptor: oxi_core::StatusDescriptor::SpecificCommandReject,
            data,
            ..
        }) = &actions[0]
        else {
            panic!("se esperaba Specific Command Reject: {actions:?}");
        };
        assert_eq!(data, &vec!["B04".to_string()]);
    }

    #[test]
    fn supply_counters_in_service_should_specific_reject_c04() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::InService);
        terminal.config_mut().options.insert(12, 1);
        let message = parse_line(b"1\x1c\x1c\x1c4");
        let actions = terminal.handle(Event::HostMessage(message));
        let Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
            descriptor: oxi_core::StatusDescriptor::SpecificCommandReject,
            data,
            ..
        }) = &actions[0]
        else {
            panic!("se esperaba Specific Command Reject: {actions:?}");
        };
        assert_eq!(data, &vec!["C04".to_string()]);
    }

    #[test]
    fn tallies_request_should_be_rejected_as_unsupported() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        let message = parse_line(b"1\x1c\x1c\x1c5A");
        let actions = terminal.handle(Event::HostMessage(message));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Send(oxi_core::TerminalMessage::SolicitedStatus {
                descriptor: oxi_core::StatusDescriptor::CommandReject,
                ..
            })
        )));
    }

    #[test]
    fn config_params_load_should_apply_timers() {
        let mut terminal = Terminal::new();
        terminal.set_mode(OperationalMode::OutOfService);
        let head = "0000000000000009000000000";
        let message = parse_line(format!("3\x1c\x1c\x1c13\x1c{head}\x1c001\x1c03030").as_bytes());
        terminal.handle(Event::HostMessage(message));
        assert_eq!(
            terminal.config().timer_duration(3),
            Some(std::time::Duration::from_secs(24))
        );
    }
}
