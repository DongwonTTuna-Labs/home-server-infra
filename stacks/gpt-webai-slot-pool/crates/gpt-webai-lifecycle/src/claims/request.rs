use serde::Serialize;

use crate::contracts::events::Writer;
use crate::contracts::ids::{validate_operation_id, validate_request_id};
use crate::contracts::projection::CasRecord;

use super::{
    derived_id, ensure_subject_available, fencing_hash, grant, CasError, CasKind, GrantInput,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestClaimPreimage<'a> {
    request_id: &'a str,
    operation_id: &'a str,
    generation: u16,
}

pub fn grant_request_claim<'a>(
    existing: impl IntoIterator<Item = &'a CasRecord>,
    request_id: &str,
    operation_id: &str,
    fencing_token: &str,
    owner: Writer,
    now_ms: u64,
    event_id: String,
) -> Result<CasRecord, CasError> {
    if validate_request_id(request_id).is_err()
        || validate_operation_id(operation_id).is_err()
        || fencing_token.is_empty()
        || fencing_token.len() > 4096
        || fencing_token.contains('\0')
    {
        return Err(CasError::Invalid("request claim input"));
    }
    ensure_subject_available(existing, request_id)?;
    let generation = 1;
    let id = derived_id(
        "claim_",
        &RequestClaimPreimage {
            request_id,
            operation_id,
            generation,
        },
    )?;
    grant(GrantInput {
        id,
        kind: CasKind::RequestClaim,
        subject_id: request_id.to_string(),
        owner,
        generation,
        fencing_token_sha256: Some(fencing_hash(fencing_token)),
        now_ms,
        event_id,
    })
}
