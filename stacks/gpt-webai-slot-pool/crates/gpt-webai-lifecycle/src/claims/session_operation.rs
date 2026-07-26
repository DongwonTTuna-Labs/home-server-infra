use serde::Serialize;

use crate::contracts::events::Writer;
use crate::contracts::ids::{validate_operation_id, validate_session_id};
use crate::contracts::projection::CasRecord;

use super::{
    derived_id, ensure_subject_available, fencing_hash, grant, CasError, CasKind, GrantInput,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionClaimPreimage<'a> {
    session_id: &'a str,
    operation_id: &'a str,
    operation_kind: &'a str,
    generation: u16,
}

pub struct SessionOperationClaimInput<'a> {
    pub session_id: &'a str,
    pub operation_id: &'a str,
    pub operation_kind: &'a str,
    pub fencing_token: &'a str,
    pub owner: Writer,
    pub now_ms: u64,
    pub event_id: String,
}

pub fn derive_session_operation_claim_id(
    session_id: &str,
    operation_id: &str,
    operation_kind: &str,
    generation: u16,
) -> Result<String, CasError> {
    if validate_session_id(session_id).is_err()
        || validate_operation_id(operation_id).is_err()
        || !matches!(operation_kind, "resume" | "show" | "download" | "poll")
        || generation == 0
    {
        return Err(CasError::Invalid("operationKind"));
    }
    derived_id(
        "claim_",
        &SessionClaimPreimage {
            session_id,
            operation_id,
            operation_kind,
            generation,
        },
    )
}

pub fn grant_session_operation_claim<'a>(
    existing: impl IntoIterator<Item = &'a CasRecord>,
    input: SessionOperationClaimInput<'_>,
) -> Result<CasRecord, CasError> {
    if validate_session_id(input.session_id).is_err()
        || validate_operation_id(input.operation_id).is_err()
        || input.fencing_token.is_empty()
        || input.fencing_token.len() > 4096
        || input.fencing_token.contains('\0')
        || !matches!(
            input.operation_kind,
            "resume" | "show" | "download" | "poll"
        )
    {
        return Err(CasError::Invalid("operationKind"));
    }
    ensure_subject_available(existing, input.session_id)?;
    let generation = 1;
    let id = derive_session_operation_claim_id(
        input.session_id,
        input.operation_id,
        input.operation_kind,
        generation,
    )?;
    grant(GrantInput {
        id,
        kind: CasKind::SessionOperationClaim,
        subject_id: input.session_id.to_string(),
        owner: input.owner,
        generation,
        fencing_token_sha256: Some(fencing_hash(input.fencing_token)),
        now_ms: input.now_ms,
        event_id: input.event_id,
    })
}
