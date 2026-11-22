//! Manejo de mensajes del host: matriz de modos (SPEC §4.5) + aplicación.
//!
//! La política vive en `oxi_core::disposition` (Tablas 10-38/39/40); aquí se
//! ejecuta: responder, aplicar la customización o rechazar con log.

use oxi_core::{Disposition, HostMessage, ModeContext, StateNumber, TerminalCommand};

use crate::event::Action;
use crate::state_exec::enter_state;
use crate::terminal::{apply_customisation, ready_action, send, Terminal};

/// Procesa un mensaje del host según el modo operativo.
pub(crate) fn handle_host(terminal: &mut Terminal, message: HostMessage) -> Vec<Action> {
    let context = ModeContext {
        mode: terminal.mode(),
        in_transaction_request: terminal.awaiting_reply(),
    };
    match oxi_core::disposition(&message, &context) {
        Disposition::NoAction => Vec::new(),
        Disposition::Reject => reject(terminal, &message),
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
            vec![
                send(terminal.command_reject()),
                Action::Log("cambio de claves: no soportado en v1 (fase 7)".into()),
            ]
        }
        Disposition::AcceptIfTranReqElseReject => vec![
            send(terminal.command_reject()),
            Action::Log("Interactive Transaction Response: no soportada en v1 (fase 8)".into()),
        ],
    }
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
        HostMessage::EncryptionKeyChange { .. }
        | HostMessage::ExtendedEncryptionKeyChange { .. } => {
            actions.push(send(terminal.command_reject()));
            actions.push(Action::Log("cambio de claves: fase 7".into()));
        }
        HostMessage::Unknown { .. } => {
            actions.push(send(terminal.command_reject()));
            actions.push(Action::Log("mensaje no modelado: reject".into()));
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
            // g1 '2' = contadores de suministros (§9-19); datos en fase 4.
            vec![send(terminal.terminal_state('2', vec![]))]
        }
        TerminalCommand::SendDateAndTime => {
            // g1 '5' = fecha y hora del terminal (§9-19); datos en fase 4.
            vec![send(terminal.terminal_state('5', vec![]))]
        }
        TerminalCommand::SendTallyInformation { .. } | TerminalCommand::SendErrorLog => {
            // No soportados en Advance NDC (manual §10-4, §9-35/§9-36).
            vec![
                send(terminal.command_reject()),
                Action::Log("tallies / error log no soportados por Advance NDC".into()),
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
    for group in reply.print_groups {
        actions.push(Action::Print {
            printer: group.flag,
            data: group.data,
        });
    }

    // Dispensado: Ready se envía cuando el cardholder retira (timer 07).
    let mut ready_deferred = false;
    if let Some(notes) = reply.notes {
        let counts = notes.counts();
        terminal.set_last_transaction(crate::terminal::LastTransaction {
            serial: reply
                .serial
                .as_deref()
                .and_then(|serial| serial.parse().ok())
                .unwrap_or(0),
            status: '1',
            notes: counts,
        });
        if notes.total() > 0 {
            terminal.set_pending(Some(crate::event::Pending::Cash));
            actions.push(Action::Dispense(notes));
            actions.push(crate::state_exec::start_timer(terminal, 7));
            ready_deferred = true;
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

/// Config ID como 4 dígitos (0000 = instalación, manual §10-33).
fn config_id_text(terminal: &Terminal) -> String {
    format!("{:04}", terminal.config_id().unwrap_or(0))
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
