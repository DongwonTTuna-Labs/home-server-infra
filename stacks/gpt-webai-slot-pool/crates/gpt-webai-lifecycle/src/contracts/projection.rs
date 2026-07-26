use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::events::Writer;
use super::ids::{
    validate_artifact_claim_id, validate_cohort, validate_event_id, validate_generation,
    validate_h256, validate_non_empty_text, validate_request_id, validate_runtime_incarnation_id,
    validate_session_id, validate_slot_id, validate_timestamp_ms,
};

pub const PROJECTION_SCHEMA: &str = "pr72.projection.r13.v1";
pub const PROJECTION_ORDER: [&str; 10] = [
    "requests",
    "sessions",
    "slots",
    "allocator",
    "claims",
    "leases",
    "runtime_owners",
    "artifact_claims",
    "releases",
    "qa_counters",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectionFile {
    pub projection_name: String,
    pub last_event_id: Option<String>,
    pub records: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RequestRecord {
    pub request_id: String,
    pub kind: String,
    pub state: String,
    pub session_id: Option<String>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionRecord {
    pub session_id: String,
    pub session_binding_id: Option<String>,
    pub conversation_url: Option<String>,
    pub slot_id: String,
    pub cohort: String,
    pub page_binding_generation: u16,
    pub last_operation_kind: Option<String>,
    pub terminal_answer_sha256: Option<String>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SlotRecord {
    pub slot_id: String,
    pub cohort: String,
    pub health_status: String,
    pub docker_status: String,
    pub allocatable: bool,
    pub cooldown_until_ms: Option<u64>,
    pub standby: bool,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct WithinCursors {
    pub cohort_a: u8,
    pub cohort_b: u8,
    pub cohort_c: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AllocatorRecord {
    pub cohort_cursor: u8,
    pub within_cursors: WithinCursors,
    pub last_scan_ordinal: Option<u8>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CasRecord {
    pub id: String,
    pub kind: String,
    pub subject_id: String,
    pub owner: Writer,
    pub generation: u16,
    pub renewal_revision: u16,
    pub fencing_token_sha256: Option<String>,
    pub granted_at_ms: u64,
    pub renew_at_ms: u64,
    pub expires_at_ms: u64,
    pub status: String,
    pub released_at_ms: Option<u64>,
    pub release_event_id: Option<String>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeOwnerRecord {
    #[serde(flatten)]
    pub cas: CasRecord,
    pub runtime_incarnation_id: String,
    pub docker_status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactClaimRecord {
    pub artifact_claim_id: String,
    pub session_id: String,
    pub request_id: Option<String>,
    pub expectation: String,
    pub control_count: Option<u8>,
    pub attempts_consumed: u8,
    pub completed: bool,
    pub result: Option<String>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReleaseRecord {
    pub release_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub reason: String,
    pub started_at_ms: u64,
    pub evidence_preserved_event_id: Option<String>,
    pub runtime_outcome: String,
    pub request_claim_release: String,
    pub session_claim_release: String,
    pub slot_lease_release: String,
    pub runtime_owner_release: String,
    pub standby_written: bool,
    pub final_status: Option<String>,
    pub finalized_at_ms: Option<u64>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct QaCounterRecord {
    pub matrix_iterations_passed: u8,
    pub repeat_counts: BTreeMap<String, u8>,
    pub source_fingerprint: Option<String>,
    pub last_reset_event_id: Option<String>,
    pub last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectionState {
    pub allocator: BTreeMap<String, AllocatorRecord>,
    pub artifact_claims: BTreeMap<String, ArtifactClaimRecord>,
    pub claims: BTreeMap<String, CasRecord>,
    pub last_event_created_at_ms: u64,
    pub last_event_id: Option<String>,
    pub leases: BTreeMap<String, CasRecord>,
    pub projection_digest: String,
    pub qa_counters: BTreeMap<String, QaCounterRecord>,
    pub releases: BTreeMap<String, ReleaseRecord>,
    pub requests: BTreeMap<String, RequestRecord>,
    pub runtime_owners: BTreeMap<String, RuntimeOwnerRecord>,
    pub schema_version: String,
    pub sessions: BTreeMap<String, SessionRecord>,
    pub slots: BTreeMap<String, SlotRecord>,
}

#[derive(Debug, Error)]
pub enum ProjectionContractError {
    #[error("unknown projection: {0}")]
    Unknown(String),
    #[error("projection record invalid: {0}")]
    Invalid(String),
}

impl ProjectionFile {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        if !PROJECTION_ORDER.contains(&self.projection_name.as_str()) {
            return Err(ProjectionContractError::Unknown(
                self.projection_name.clone(),
            ));
        }
        if self.records.is_empty() != self.last_event_id.is_none()
            || self
                .last_event_id
                .as_deref()
                .is_some_and(|id| validate_event_id(id).is_err())
        {
            return Err(ProjectionContractError::Invalid(
                self.projection_name.clone(),
            ));
        }
        for (key, value) in &self.records {
            validate_record(&self.projection_name, key, value)?;
        }
        Ok(())
    }
}

fn validate_record(name: &str, key: &str, value: &Value) -> Result<(), ProjectionContractError> {
    match name {
        "requests" => validate_request(key, parse(value)?),
        "sessions" => validate_session(key, parse(value)?),
        "slots" => validate_slot(key, parse(value)?),
        "allocator" => validate_allocator(key, parse(value)?),
        "claims" | "leases" => validate_cas(name, key, &parse(value)?),
        "runtime_owners" => validate_owner(key, parse(value)?),
        "artifact_claims" => validate_artifact_claim(key, parse(value)?),
        "releases" => validate_release(key, parse(value)?),
        "qa_counters" => validate_qa(key, parse(value)?),
        _ => Err(ProjectionContractError::Unknown(name.to_string())),
    }
}

pub fn validate_projection_digest(value: &str) -> Result<(), ProjectionContractError> {
    validate_h256(value).map_err(|error| ProjectionContractError::Invalid(error.to_string()))
}

impl ProjectionState {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        if self.schema_version != PROJECTION_SCHEMA
            || validate_h256(&self.projection_digest).is_err()
            || self.last_event_id.is_none() != (self.last_event_created_at_ms == 0)
            || self
                .last_event_id
                .as_deref()
                .is_some_and(|id| validate_event_id(id).is_err())
            || (self.last_event_created_at_ms != 0
                && validate_timestamp_ms(self.last_event_created_at_ms).is_err())
        {
            return invalid("ProjectionState metadata");
        }
        validate_typed_map("requests", &self.requests)?;
        validate_typed_map("sessions", &self.sessions)?;
        validate_typed_map("slots", &self.slots)?;
        validate_typed_map("allocator", &self.allocator)?;
        validate_typed_map("claims", &self.claims)?;
        validate_typed_map("leases", &self.leases)?;
        validate_typed_map("runtime_owners", &self.runtime_owners)?;
        validate_typed_map("artifact_claims", &self.artifact_claims)?;
        validate_typed_map("releases", &self.releases)?;
        validate_typed_map("qa_counters", &self.qa_counters)
    }
}

fn validate_typed_map<T: Serialize>(
    name: &str,
    records: &BTreeMap<String, T>,
) -> Result<(), ProjectionContractError> {
    for (key, value) in records {
        validate_record(
            name,
            key,
            &serde_json::to_value(value)
                .map_err(|error| ProjectionContractError::Invalid(error.to_string()))?,
        )?;
    }
    Ok(())
}

fn validate_request(key: &str, record: RequestRecord) -> Result<(), ProjectionContractError> {
    if key != record.request_id
        || validate_request_id(key).is_err()
        || !matches!(record.kind.as_str(), "pro" | "xhigh")
        || !matches!(
            record.state.as_str(),
            "accepted"
                | "claimed"
                | "allocated"
                | "binding"
                | "model_verified"
                | "uploading"
                | "send_armed"
                | "sent"
                | "running"
                | "polling"
                | "terminal"
                | "output_published"
                | "failed"
                | "released"
        )
        || record
            .session_id
            .as_deref()
            .is_some_and(|value| validate_session_id(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("RequestRecord");
    }
    Ok(())
}

fn validate_session(key: &str, record: SessionRecord) -> Result<(), ProjectionContractError> {
    if key != record.session_id
        || validate_session_id(key).is_err()
        || record
            .session_binding_id
            .as_deref()
            .is_some_and(|value| super::ids::validate_binding_id(value).is_err())
        || record.conversation_url.as_deref().is_none_or(|value| {
            super::ids::validate_conversation_url(value, &record.session_id).is_err()
        })
        || validate_slot_id(&record.slot_id).is_err()
        || validate_cohort(&record.cohort).is_err()
        || super::ids::validate_generation(record.page_binding_generation).is_err()
        || record
            .last_operation_kind
            .as_deref()
            .is_some_and(|value| validate_non_empty_text(value).is_err())
        || record
            .terminal_answer_sha256
            .as_deref()
            .is_some_and(|value| validate_h256(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("SessionRecord");
    }
    Ok(())
}

fn validate_slot(key: &str, record: SlotRecord) -> Result<(), ProjectionContractError> {
    if key != record.slot_id
        || validate_slot_id(key).is_err()
        || validate_cohort(&record.cohort).is_err()
        || crate::allocator::cohort_of(key) != Some(record.cohort.as_str())
        || crate::contracts::health::HealthStatus::parse(&record.health_status).is_none()
        || !docker_status(&record.docker_status)
        || record
            .cooldown_until_ms
            .is_some_and(|value| validate_timestamp_ms(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("SlotRecord");
    }
    Ok(())
}

fn validate_allocator(key: &str, record: AllocatorRecord) -> Result<(), ProjectionContractError> {
    if key != "allocator"
        || record.cohort_cursor > 2
        || record.within_cursors.cohort_a > 2
        || record.within_cursors.cohort_b > 3
        || record.within_cursors.cohort_c > 2
        || record.last_scan_ordinal.is_some_and(|value| value > 9)
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("AllocatorRecord");
    }
    Ok(())
}

fn validate_cas(
    projection: &str,
    key: &str,
    record: &CasRecord,
) -> Result<(), ProjectionContractError> {
    let identity = match projection {
        "claims" => super::ids::validate_claim_id(key),
        "leases" => super::ids::validate_lease_id(key),
        "runtime_owners" => super::ids::validate_owner_id(key),
        _ => return invalid("CAS projection"),
    };
    let released = record.status == "released";
    if identity.is_err()
        || key != record.id
        || record.kind != projection_kind(projection)
        || validate_generation(record.generation).is_err()
        || record.renewal_revision == 0
        || validate_timestamp_ms(record.granted_at_ms).is_err()
        || validate_timestamp_ms(record.renew_at_ms).is_err()
        || validate_timestamp_ms(record.expires_at_ms).is_err()
        || record.renew_at_ms + 200_000 != record.expires_at_ms
        || record.granted_at_ms > record.renew_at_ms
        || !matches!(record.status.as_str(), "active" | "released")
        || (record.released_at_ms.is_some() != released)
        || (record.release_event_id.is_some() != released)
        || record
            .released_at_ms
            .is_some_and(|value| validate_timestamp_ms(value).is_err())
        || record
            .release_event_id
            .as_deref()
            .is_some_and(|value| validate_event_id(value).is_err())
        || record
            .fencing_token_sha256
            .as_deref()
            .is_some_and(|value| validate_h256(value).is_err())
        || projection != "runtime_owners" && record.fencing_token_sha256.is_none()
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("CasRecord");
    }
    Ok(())
}

fn validate_owner(key: &str, record: RuntimeOwnerRecord) -> Result<(), ProjectionContractError> {
    validate_cas("runtime_owners", key, &record.cas)?;
    if validate_slot_id(&record.cas.subject_id).is_err()
        || validate_runtime_incarnation_id(&record.runtime_incarnation_id).is_err()
        || !docker_status(&record.docker_status)
    {
        return invalid("RuntimeOwnerRecord");
    }
    Ok(())
}

fn validate_artifact_claim(
    key: &str,
    record: ArtifactClaimRecord,
) -> Result<(), ProjectionContractError> {
    if key != record.artifact_claim_id
        || validate_artifact_claim_id(key).is_err()
        || validate_session_id(&record.session_id).is_err()
        || record
            .request_id
            .as_deref()
            .is_some_and(|value| validate_request_id(value).is_err())
        || !matches!(
            record.expectation.as_str(),
            "none" | "optional" | "required" | "claimed"
        )
        || record.control_count.is_some_and(|value| value > 64)
        || record.attempts_consumed > 64
        || (record.completed != record.result.is_some() && record.result.is_some())
        || record
            .result
            .as_deref()
            .is_some_and(|value| validate_non_empty_text(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("ArtifactClaimRecord");
    }
    Ok(())
}

fn validate_release(key: &str, record: ReleaseRecord) -> Result<(), ProjectionContractError> {
    let final_pair = record.final_status.is_some() == record.finalized_at_ms.is_some();
    if key != record.release_id
        || super::ids::validate_release_id(key).is_err()
        || !matches!(
            record.subject_kind.as_str(),
            "request" | "session_operation" | "slot"
        )
        || validate_non_empty_text(&record.subject_id).is_err()
        || validate_non_empty_text(&record.reason).is_err()
        || validate_timestamp_ms(record.started_at_ms).is_err()
        || record
            .evidence_preserved_event_id
            .as_deref()
            .is_some_and(|value| validate_event_id(value).is_err())
        || !matches!(
            record.runtime_outcome.as_str(),
            "pending" | "stopped" | "skipped" | "failed"
        )
        || !release_mode(&record.request_claim_release)
        || !release_mode(&record.session_claim_release)
        || !release_mode(&record.slot_lease_release)
        || !release_mode(&record.runtime_owner_release)
        || !final_pair
        || record.final_status.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "allocatable"
                    | "cooldown_blocked"
                    | "cleanup_failed"
                    | "stop_skipped_owner_alive"
                    | "resources_released_no_slot"
            )
        })
        || record
            .finalized_at_ms
            .is_some_and(|value| validate_timestamp_ms(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("ReleaseRecord");
    }
    Ok(())
}

fn validate_qa(key: &str, record: QaCounterRecord) -> Result<(), ProjectionContractError> {
    if key != "qa"
        || record.matrix_iterations_passed > 3
        || record
            .repeat_counts
            .iter()
            .any(|(case, value)| validate_non_empty_text(case).is_err() || *value > 10)
        || record
            .source_fingerprint
            .as_deref()
            .is_some_and(|value| validate_h256(value).is_err())
        || record
            .last_reset_event_id
            .as_deref()
            .is_some_and(|value| validate_event_id(value).is_err())
        || validate_event_id(&record.last_event_id).is_err()
    {
        return invalid("QaCounterRecord");
    }
    Ok(())
}

fn parse<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, ProjectionContractError> {
    serde_json::from_value(value.clone())
        .map_err(|error| ProjectionContractError::Invalid(error.to_string()))
}

fn projection_kind(projection: &str) -> &'static str {
    match projection {
        "claims" => "claim",
        "leases" => "lease",
        "runtime_owners" => "runtime_owner",
        _ => "invalid",
    }
}

fn docker_status(value: &str) -> bool {
    matches!(
        value,
        "running" | "exited" | "missing" | "starting" | "stopping" | "unknown"
    )
}

fn release_mode(value: &str) -> bool {
    matches!(value, "not_applicable" | "pending" | "released")
}

fn invalid<T>(message: &str) -> Result<T, ProjectionContractError> {
    Err(ProjectionContractError::Invalid(message.to_string()))
}
