use thiserror::Error;

use crate::provider_runner::ProviderCommand;
use crate::visual_gate::{confirm_pre_send_visual_gate, VisualGateError};

use super::input::RequestRunInput;
use super::provider::{provider_capture, provider_status};

#[derive(Debug, Error)]
pub enum RequestVisualGateError {
    #[error("provider status failed: {0}")]
    Status(String),
    #[error("provider capture failed: {0}")]
    Capture(String),
    #[error("visual gate failed: {0}")]
    Gate(#[from] VisualGateError),
}

pub(crate) fn run_pre_send_visual_gate(
    input: &RequestRunInput,
    command: &ProviderCommand,
) -> Result<(), RequestVisualGateError> {
    let mut retry_index = 0;
    loop {
        match run_pre_send_visual_gate_once(input, command) {
            Ok(()) => return Ok(()),
            Err(error) if retryable_visual_gate_error(&error) => {
                let Some(delay) = input.send_retry_delays.get(retry_index).copied() else {
                    return Err(error);
                };
                retry_index += 1;
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn run_pre_send_visual_gate_once(
    input: &RequestRunInput,
    command: &ProviderCommand,
) -> Result<(), RequestVisualGateError> {
    let status = provider_status(input, command)
        .map_err(|error| RequestVisualGateError::Status(error.to_string()))?;
    let capture = provider_capture(input, command, "pre-send-visual-gate")
        .map_err(|error| RequestVisualGateError::Capture(error.to_string()))?;
    confirm_pre_send_visual_gate(&status.value, &capture.value)?;
    Ok(())
}

fn retryable_visual_gate_error(error: &RequestVisualGateError) -> bool {
    match error {
        RequestVisualGateError::Status(_) | RequestVisualGateError::Capture(_) => true,
        RequestVisualGateError::Gate(VisualGateError::NotReady(status)) => {
            matches!(status.as_str(), "unknown" | "unreachable")
        }
        RequestVisualGateError::Gate(
            VisualGateError::CaptureMissing | VisualGateError::StatusDiagnosticsMissing,
        ) => true,
        RequestVisualGateError::Gate(VisualGateError::ReadinessSignal("composer")) => true,
        RequestVisualGateError::Gate(_) => false,
    }
}
