pub mod renewal;
pub mod request;
pub mod session_operation;

use serde::Serialize;
use thiserror::Error;

use crate::contracts::events::Writer;
use crate::contracts::ids::{
    h256, validate_claim_id, validate_event_id, validate_h256, validate_lease_id, validate_owner_id,
};
use crate::contracts::projection::CasRecord;
use crate::journal::canonical::canonical_bytes;

pub const RESOURCE_TTL_MS: u64 = 300_000;
pub const RENEW_CADENCE_MS: u64 = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CasKind {
    RequestClaim,
    SessionOperationClaim,
    SlotLease,
    RuntimeOwner,
}

impl CasKind {
    pub fn record_kind(self) -> &'static str {
        match self {
            Self::RequestClaim | Self::SessionOperationClaim => "claim",
            Self::SlotLease => "lease",
            Self::RuntimeOwner => "runtime_owner",
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CasError {
    #[error("resource already active for subject: {0}")]
    SubjectConflict(String),
    #[error("resource is not active")]
    Inactive,
    #[error("resource generation mismatch")]
    GenerationMismatch,
    #[error("resource renewal expired")]
    Expired,
    #[error("resource fencing token mismatch")]
    FencingMismatch,
    #[error("resource contract invalid: {0}")]
    Invalid(&'static str),
}

#[derive(Clone, Debug)]
pub struct GrantInput {
    pub id: String,
    pub kind: CasKind,
    pub subject_id: String,
    pub owner: Writer,
    pub generation: u16,
    pub fencing_token_sha256: Option<String>,
    pub now_ms: u64,
    pub event_id: String,
}

pub fn grant(input: GrantInput) -> Result<CasRecord, CasError> {
    let id_valid = match input.kind {
        CasKind::RequestClaim | CasKind::SessionOperationClaim => validate_claim_id(&input.id),
        CasKind::SlotLease => validate_lease_id(&input.id),
        CasKind::RuntimeOwner => validate_owner_id(&input.id),
    };
    if id_valid.is_err()
        || input.generation == 0
        || input.now_ms == 0
        || validate_event_id(&input.event_id).is_err()
    {
        return Err(CasError::Invalid("grant identity/time"));
    }
    if input
        .fencing_token_sha256
        .as_deref()
        .is_some_and(|value| validate_h256(value).is_err())
    {
        return Err(CasError::Invalid("fencingTokenSha256"));
    }
    Ok(CasRecord {
        id: input.id,
        kind: input.kind.record_kind().to_string(),
        subject_id: input.subject_id,
        owner: input.owner,
        generation: input.generation,
        renewal_revision: 1,
        fencing_token_sha256: input.fencing_token_sha256,
        granted_at_ms: input.now_ms,
        renew_at_ms: input.now_ms + RENEW_CADENCE_MS,
        expires_at_ms: input.now_ms + RESOURCE_TTL_MS,
        status: "active".to_string(),
        released_at_ms: None,
        release_event_id: None,
        last_event_id: input.event_id,
    })
}

pub fn ensure_subject_available<'a>(
    records: impl IntoIterator<Item = &'a CasRecord>,
    subject_id: &str,
) -> Result<(), CasError> {
    records
        .into_iter()
        .any(|record| record.subject_id == subject_id && record.status == "active")
        .then(|| CasError::SubjectConflict(subject_id.to_string()))
        .map_or(Ok(()), Err)
}

pub fn fencing_hash(token: &str) -> String {
    h256(token.as_bytes())
}

pub fn derived_id(prefix: &str, value: &impl Serialize) -> Result<String, CasError> {
    canonical_bytes(value)
        .map(|bytes| format!("{prefix}{}", crate::contracts::ids::sha256_hex(bytes)))
        .map_err(|_| CasError::Invalid("id preimage"))
}
