use thiserror::Error;

use crate::allocator::scan::ScanError;
use crate::contracts::cli::CommandOutcomeError;
use crate::provider_runner::ProviderRunnerError;
use crate::sessions::SessionRecordError;

use super::r13_assets::FreshAssetError;
use super::r13_events::SendEventError;
use super::r13_provider::FreshProviderError;

#[derive(Debug, Error)]
pub enum FreshRunError {
    #[error("fresh run asset failed: {0}")]
    Asset(#[from] FreshAssetError),
    #[error("fresh run journal failed: {0}")]
    Journal(#[from] crate::session_ops::journal::SessionJournalError),
    #[error("fresh run scan failed: {0}")]
    Scan(#[from] ScanError),
    #[error("fresh run provider command failed: {0}")]
    ProviderCommand(#[from] ProviderRunnerError),
    #[error("fresh run runtime failed: {0}")]
    Runtime(#[from] crate::session_ops::runtime_r13::SessionRuntimeR13Error),
    #[error("fresh run provider failed: {0}")]
    Provider(#[from] FreshProviderError),
    #[error("fresh run provider status failed: {0}")]
    StatusProvider(#[from] crate::session_ops::provider::RebindProviderError),
    #[error("fresh run session rebind failed: {0}")]
    Rebind(#[from] crate::session_rebind::SessionRebindError),
    #[error("fresh run release failed: {0}")]
    Release(#[from] crate::session_ops::release::SessionReleaseError),
    #[error("fresh run identifier derivation failed: {0}")]
    Id(#[from] crate::claims::CasError),
    #[error("fresh run JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fresh run event failed: {0}")]
    Event(#[from] SendEventError),
    #[error("fresh run session failed: {0}")]
    Session(#[from] SessionRecordError),
    #[error("fresh run output failed: {0}")]
    Outcome(#[from] CommandOutcomeError),
    #[error("fresh run terminal pipeline failed: {0}")]
    Terminal(#[from] crate::session_ops::terminal::TerminalPipelineError),
    #[error("fresh run contract failed: {0}")]
    Contract(&'static str),
}

pub fn child_operation_id(parent: &str, suffix: &str) -> Result<String, FreshRunError> {
    let value = format!("{parent}.{suffix}");
    crate::contracts::ids::validate_operation_id(&value)
        .map_err(|_| FreshRunError::Contract("operationId"))?;
    Ok(value)
}
