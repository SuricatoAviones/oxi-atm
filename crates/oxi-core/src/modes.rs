//! Matriz de modos operativos: qué hace el terminal con cada mensaje según su
//! modo (SPEC §4.5; manual §10-95…§10-97, Tablas 10-38/39/40).
//!
//! Esta función es POLÍTICA pura: dado el mensaje y el modo, dice qué
//! disposición aplica; el motor (fase 3) ejecuta la acción (enviar reject,
//! retener, cambiar de modo…). Nota del manual (§10-97): en In-Service los
//! comandos se retienen hasta quedar inactivo y NO se apilan — uno nuevo
//! pisa al anterior sin respuesta; eso es secuencia del motor, no layout.

use crate::command::TerminalCommand;
use crate::customisation::CustomisationData;
use crate::host::HostMessage;

/// Modo operativo del terminal (manual §10-95).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalMode {
    /// Recién energizado, sin customización verificada.
    PowerUp,
    /// Fuera de servicio.
    OutOfService,
    /// Modo de reabastecimiento/supervisión.
    Supply,
    /// En servicio (transacciones de cardholder).
    InService,
}

/// Contexto que el motor aporta además del modo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeContext {
    /// Modo actual del terminal.
    pub mode: OperationalMode,
    /// Solo relevante en In-Service: ¿está el terminal esperando el
    /// Transaction Reply? (Tabla 10-39 divide In-Service en "Not Tran. Req.
    /// State" y "Tran. Req. State").
    pub in_transaction_request: bool,
}

impl ModeContext {
    /// Constructor breve.
    pub fn new(mode: OperationalMode) -> Self {
        Self {
            mode,
            in_transaction_request: false,
        }
    }
}

/// Disposición ante un mensaje según modo (leyendas de las Tablas 10-38/39/40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// '–': sin acción (Power-Up ignora la mayoría de los comandos).
    NoAction,
    /// 'R': responder Command Reject.
    Reject,
    /// 'A': procesar y responder Ready ('9').
    AcceptReady,
    /// 'A1' (Tabla 10-40, Go In-Service desde OOS): cambiar a In-Service y
    /// responder Ready.
    ChangeToInService,
    /// 'A2'/'P' (Tablas 10-38/10-40): retener hasta salir de Supply y
    /// procesar según el modo que se entre. Solo el último se reconoce; los
    /// anteriores se descartan sin respuesta.
    HoldUntilSupplyExitLastWins,
    /// 'A3' (Tabla 10-40, Go Out-of-Service en In-Service): cambiar a
    /// Out-of-Service cuando esté inactivo en el estado Card Read;
    /// responder Ready.
    ChangeToOutOfServiceIdle,
    /// 'A1' (Tabla 10-39, Print Immediate desde Supply): retener hasta
    /// re-entrar Out-of-Service y procesar como A; si llega otro, pisa al
    /// primero sin respuesta adicional.
    HoldUntilOutOfService,
    /// 'A3' (Tabla 10-38, EKC en In-Service): aceptar solo si NO hay
    /// transacción en curso.
    AcceptOnlyIfNoTransaction,
    /// 'D': responder con la información pedida.
    RespondRequestedInfo,
    /// 'C': responder con el Terminal State que contiene el Config ID.
    RespondConfigId,
    /// 'A2' (Tabla 10-38, ITR en In-Service): aceptar solo si se está en el
    /// estado Transaction Request; si no, Command Reject.
    AcceptIfTranReqElseReject,
}

/// Disposición del mensaje según el modo (Tablas 10-38/39/40).
pub fn disposition(message: &HostMessage, ctx: &ModeContext) -> Disposition {
    use CustomisationData as Cd;
    use Disposition as D;
    use HostMessage as H;
    use OperationalMode as M;

    match message {
        // Tabla 10-38: customización. PU '–' / OOS 'A' / Supply 'A' / IS 'R'
        // (la ITR es la excepción, con 'A2' en In-Service).
        H::Customisation(Cd::Screens(_))
        | H::Customisation(Cd::StateTables(_))
        | H::Customisation(Cd::ConfigParams(_))
        | H::Customisation(Cd::Fits(_))
        | H::Customisation(Cd::ConfigId(_))
        | H::Customisation(Cd::EnhancedConfig(_))
        | H::Customisation(Cd::MacFieldSelection(_))
        | H::Customisation(Cd::DateTime(_))
        | H::Customisation(Cd::CassetteMapping(_))
        | H::Customisation(Cd::OverrideReservedScreens(_)) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService | M::Supply => D::AcceptReady,
            M::InService => D::Reject,
        },

        // Tabla 10-38: ITR. PU '–' / OOS 'R' / Supply 'R' / IS 'A2'.
        H::InteractiveTransactionResponse(_) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService | M::Supply => D::Reject,
            M::InService => D::AcceptIfTranReqElseReject,
        },

        // Tabla 10-38: EKC y Extended. IS 'A3': solo sin transacción.
        H::EncryptionKeyChange { .. } | H::ExtendedEncryptionKeyChange { .. } => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService | M::Supply => D::AcceptReady,
            M::InService => D::AcceptOnlyIfNoTransaction,
        },

        // Tabla 10-39: Transaction Reply. Print Immediate (fn '4') se
        // distingue del resto; In-Service se divide por estado.
        H::TransactionReply(reply) => {
            let print_immediate = reply.function_id == Some('4');
            match (ctx.mode, ctx.in_transaction_request, print_immediate) {
                (M::PowerUp, _, true) => D::NoAction,
                (M::PowerUp, _, false) => D::Reject,
                // Tabla 10-39: en OOS solo Print Immediate se acepta ('A');
                // el resto de los TR se rechaza ('R').
                (M::OutOfService, _, true) => D::AcceptReady,
                (M::OutOfService, _, false) => D::Reject,
                (M::Supply, _, true) => D::HoldUntilOutOfService,
                (M::Supply, _, false) => D::Reject,
                (M::InService, false, _) => D::Reject,
                (M::InService, true, _) => D::AcceptReady,
            }
        }

        // Tabla 10-40: comandos del terminal.
        H::TerminalCommand(TerminalCommand::GoInService) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService => D::ChangeToInService,
            M::Supply => D::HoldUntilSupplyExitLastWins,
            M::InService => D::AcceptReady,
        },
        H::TerminalCommand(TerminalCommand::GoOutOfService { .. }) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService => D::AcceptReady,
            M::Supply => D::HoldUntilSupplyExitLastWins,
            M::InService => D::ChangeToOutOfServiceIdle,
        },
        H::TerminalCommand(TerminalCommand::SendConfigurationId) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService | M::Supply | M::InService => D::RespondConfigId,
        },
        H::TerminalCommand(TerminalCommand::SendConfigurationInformation { .. }) => {
            match ctx.mode {
                M::PowerUp => D::NoAction,
                M::OutOfService | M::Supply | M::InService => D::RespondRequestedInfo,
            }
        }
        H::TerminalCommand(TerminalCommand::SendSupplyCounters)
        | H::TerminalCommand(TerminalCommand::SendTallyInformation { .. })
        | H::TerminalCommand(TerminalCommand::SendErrorLog)
        | H::TerminalCommand(TerminalCommand::SendDateAndTime) => match ctx.mode {
            M::PowerUp => D::NoAction,
            M::OutOfService => D::RespondRequestedInfo,
            M::Supply => D::HoldUntilSupplyExitLastWins,
            M::InService => D::Reject,
        },
        // 'F'/'G' son solo dial-up VISA II (Ap. O): fuera de alcance en TCP.
        H::TerminalCommand(TerminalCommand::Disconnect)
        | H::TerminalCommand(TerminalCommand::NoOp) => D::NoAction,

        // Clase/código no documentado: rechazo conservador (§10-87); el
        // motor loguea el crudo para diagnóstico.
        H::Unknown { .. } => D::Reject,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StateNumber;
    use crate::wire::RawFields;
    use crate::StateTableDef;

    fn screens_msg() -> HostMessage {
        HostMessage::Customisation(CustomisationData::Screens(Vec::new()))
    }

    /// Tabla 10-38: la customización se acepta en OOS/Supply y se rechaza
    /// en In-Service (el driver real manda el download antes del In-Service).
    #[test]
    fn customisation_should_be_rejected_in_service() {
        let msg = screens_msg();
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::InService)),
            Disposition::Reject
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::OutOfService)),
            Disposition::AcceptReady
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::PowerUp)),
            Disposition::NoAction
        );
    }

    /// Tabla 10-38: ITR 'A2' en In-Service — aceptar solo en estado I.
    #[test]
    fn itr_should_require_transaction_request_in_service() {
        let msg =
            HostMessage::InteractiveTransactionResponse(crate::InteractiveTransactionResponse {
                display_flag: '1',
                active_keys: "0".into(),
                screen_timer: None,
                screen_data: Vec::new(),
            });
        let mut ctx = ModeContext::new(OperationalMode::InService);
        ctx.in_transaction_request = true;
        assert_eq!(
            disposition(&msg, &ctx),
            Disposition::AcceptIfTranReqElseReject
        );
        ctx.in_transaction_request = false;
        assert_eq!(
            disposition(&msg, &ctx),
            Disposition::AcceptIfTranReqElseReject
        );
        // La distinción true/false la resuelve el motor; la disposición es
        // la misma per tabla (la leyenda A2 indica el rechazo si no está en I).
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::Supply)),
            Disposition::Reject
        );
    }

    /// Tabla 10-38: EKC 'A3' en In-Service (solo sin transacción).
    #[test]
    fn ekc_should_be_accept_ready_out_of_service() {
        let msg = HostMessage::EncryptionKeyChange {
            raw: RawFields::parse(b"3\x1c\x1c\x1c31"),
        };
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::OutOfService)),
            Disposition::AcceptReady
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::InService)),
            Disposition::AcceptOnlyIfNoTransaction
        );
    }

    /// Tabla 10-39: TR aceptado SOLO en In-Service + estado I.
    #[test]
    fn transaction_reply_should_require_in_service_request_state() {
        let reply = crate::TransactionReply {
            next_state: StateNumber::new(10).expect("en rango"),
            notes: None,
            serial: None,
            function_id: Some('2'),
            screen: None,
            screen_display_update: None,
            mcn: Some('0'),
            card_return: None,
            print_groups: Vec::new(),
            extra: Vec::new(),
        };
        let msg = HostMessage::TransactionReply(reply);
        let mut ctx = ModeContext::new(OperationalMode::InService);
        ctx.in_transaction_request = true;
        assert_eq!(disposition(&msg, &ctx), Disposition::AcceptReady);
        ctx.in_transaction_request = false;
        assert_eq!(disposition(&msg, &ctx), Disposition::Reject);
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::OutOfService)),
            Disposition::Reject
        );
    }

    /// Tabla 10-39: Print Immediate (fn '4') se retiene si llega en Supply.
    #[test]
    fn print_immediate_should_hold_in_supply() {
        let reply = crate::TransactionReply {
            next_state: StateNumber::new(10).expect("en rango"),
            notes: None,
            serial: None,
            function_id: Some('4'),
            screen: None,
            screen_display_update: None,
            mcn: Some('0'),
            card_return: None,
            print_groups: Vec::new(),
            extra: Vec::new(),
        };
        let msg = HostMessage::TransactionReply(reply);
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::Supply)),
            Disposition::HoldUntilOutOfService
        );
        let mut ctx = ModeContext::new(OperationalMode::InService);
        ctx.in_transaction_request = true;
        assert_eq!(disposition(&msg, &ctx), Disposition::AcceptReady);
    }

    /// Tabla 10-40: Go In-Service cambia de modo desde OOS ('A1').
    #[test]
    fn go_in_service_should_change_mode_from_out_of_service() {
        let msg = HostMessage::TerminalCommand(TerminalCommand::GoInService);
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::OutOfService)),
            Disposition::ChangeToInService
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::Supply)),
            Disposition::HoldUntilSupplyExitLastWins
        );
    }

    /// Tabla 10-40: Config ID se responde con Terminal State ('C') siempre.
    #[test]
    fn config_id_request_should_respond_config_id_in_all_modes() {
        let msg = HostMessage::TerminalCommand(TerminalCommand::SendConfigurationId);
        for mode in [
            OperationalMode::OutOfService,
            OperationalMode::Supply,
            OperationalMode::InService,
        ] {
            assert_eq!(
                disposition(&msg, &ModeContext::new(mode)),
                Disposition::RespondConfigId
            );
        }
    }

    /// Tabla 10-40: los contadores ('D'/'P'/'R') — rechazo en In-Service.
    #[test]
    fn counters_should_be_rejected_in_service() {
        let msg = HostMessage::TerminalCommand(TerminalCommand::SendSupplyCounters);
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::InService)),
            Disposition::Reject
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::Supply)),
            Disposition::HoldUntilSupplyExitLastWins
        );
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::OutOfService)),
            Disposition::RespondRequestedInfo
        );
    }

    /// Estado tipado de las state tables para el caso Unknown.
    #[test]
    fn unknown_should_be_rejected_everywhere() {
        let msg = HostMessage::Unknown {
            raw: RawFields::parse(b"9\x1c\x1c\x1c1"),
        };
        for mode in [
            OperationalMode::PowerUp,
            OperationalMode::OutOfService,
            OperationalMode::Supply,
            OperationalMode::InService,
        ] {
            assert_eq!(
                disposition(&msg, &ModeContext::new(mode)),
                Disposition::Reject
            );
        }
    }

    /// Constructor de state table para futuras extensiones de la matriz.
    #[test]
    fn state_tables_should_reject_in_service_too() {
        let msg = HostMessage::Customisation(CustomisationData::StateTables(vec![StateTableDef {
            number: StateNumber::new(0).expect("en rango"),
            state_type: 'A',
            entries: String::new(),
        }]));
        assert_eq!(
            disposition(&msg, &ModeContext::new(OperationalMode::InService)),
            Disposition::Reject
        );
    }
}
