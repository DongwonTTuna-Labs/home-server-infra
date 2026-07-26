use crate::contracts::browser::PageBindingEcho;
use crate::contracts::ids::{validate_h256, validate_operation_id};

use super::{validate_receipt_pair, SendReceipt, SendReceiptKind, SendReconcileError, TurnStart};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArmedSend {
    pub send_attempt_id: String,
    pub page_binding: PageBindingEcho,
    pub prompt_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendState {
    FreshlyArmed(ArmedSend),
    ClickInFlight(ArmedSend),
    RecoveringArmed(ArmedSend),
    ReconcileInFlight(ArmedSend),
    Clicked { armed: ArmedSend, turn: TurnStart },
    Reconciled { armed: ArmedSend, turn: TurnStart },
    Uncertain(ArmedSend),
    Failed(ArmedSend),
}

impl ArmedSend {
    pub fn validate(&self) -> Result<(), SendReconcileError> {
        validate_operation_id(&self.send_attempt_id)
            .map_err(|_| SendReconcileError::Invalid("sendAttemptId"))?;
        self.page_binding
            .validate()
            .map_err(|_| SendReconcileError::Invalid("pageBinding"))?;
        validate_h256(&self.prompt_sha256).map_err(|_| SendReconcileError::Invalid("promptSha256"))
    }
}

impl SendState {
    pub fn freshly_armed(armed: ArmedSend) -> Result<Self, SendReconcileError> {
        armed.validate()?;
        Ok(Self::FreshlyArmed(armed))
    }

    pub fn recover_armed(armed: ArmedSend) -> Result<Self, SendReconcileError> {
        armed.validate()?;
        Ok(Self::RecoveringArmed(armed))
    }

    pub fn begin_physical_click(self) -> Result<Self, SendReconcileError> {
        match self {
            Self::FreshlyArmed(armed) => Ok(Self::ClickInFlight(armed)),
            _ => Err(SendReconcileError::IllegalTransition),
        }
    }

    pub fn begin_reconcile(self) -> Result<Self, SendReconcileError> {
        match self {
            Self::RecoveringArmed(armed) => Ok(Self::ReconcileInFlight(armed)),
            _ => Err(SendReconcileError::IllegalTransition),
        }
    }

    pub fn accept_terminal(
        self,
        pre_click: &SendReceipt,
        terminal: &SendReceipt,
        observed_binding: &PageBindingEcho,
    ) -> Result<Self, SendReconcileError> {
        let (armed, expected_kind) = match self {
            Self::ClickInFlight(armed) => (armed, SendReceiptKind::PostClick),
            Self::ReconcileInFlight(armed) => (armed, SendReceiptKind::ReconciledTurnStart),
            _ => return Err(SendReconcileError::IllegalTransition),
        };
        if terminal.kind != expected_kind
            || observed_binding != &armed.page_binding
            || pre_click.send_attempt_id != armed.send_attempt_id
            || pre_click.prompt_sha256 != armed.prompt_sha256
        {
            return Err(SendReconcileError::Invalid("terminal response binding"));
        }
        let turn = validate_receipt_pair(pre_click, terminal, &armed.page_binding)?;
        Ok(match expected_kind {
            SendReceiptKind::PostClick => Self::Clicked { armed, turn },
            SendReceiptKind::ReconciledTurnStart => Self::Reconciled { armed, turn },
            SendReceiptKind::PreClick => unreachable!(),
        })
    }

    pub fn mark_uncertain(self) -> Result<Self, SendReconcileError> {
        match self {
            Self::ReconcileInFlight(armed) => Ok(Self::Uncertain(armed)),
            _ => Err(SendReconcileError::IllegalTransition),
        }
    }

    pub fn mark_failed(self) -> Result<Self, SendReconcileError> {
        match self {
            Self::ClickInFlight(armed) | Self::ReconcileInFlight(armed) => Ok(Self::Failed(armed)),
            _ => Err(SendReconcileError::IllegalTransition),
        }
    }

    pub fn turn_start(&self) -> Option<&TurnStart> {
        match self {
            Self::Clicked { turn, .. } | Self::Reconciled { turn, .. } => Some(turn),
            _ => None,
        }
    }
}
