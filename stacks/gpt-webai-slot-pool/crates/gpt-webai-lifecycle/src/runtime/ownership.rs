use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::claims::{derived_id, fencing_hash, grant, CasError, CasKind, GrantInput};
use crate::config::{load_or_create_host_id, HostIdError};
use crate::contracts::browser::EvidenceRef;
use crate::contracts::events::Writer;
use crate::contracts::ids::{
    derive_writer_id, validate_generation, validate_owner_id, validate_runtime_incarnation_id,
    validate_slot_id, validate_timestamp_ms, IdError,
};
use crate::contracts::projection::{CasRecord, RuntimeOwnerRecord};

pub const TAKEOVER_GRACE_MS: u64 = 30_000;

pub fn generate_incarnation_nonce() -> Result<String, RuntimeIdentityError> {
    let mut entropy = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    Ok(entropy.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeadOwnerProof {
    pub prior_owner_id: String,
    pub prior_generation: u16,
    pub expired_at_ms: u64,
    pub grace_satisfied_at_ms: u64,
    pub process_absent: bool,
    pub container_label_owner_id: Option<String>,
    pub container_label_generation: Option<u16>,
    pub lease_inactive: bool,
    pub claim_inactive: bool,
    pub evidence_refs: Vec<EvidenceRef>,
    pub proven_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdoptionProof {
    pub container_label_owner_id: Option<String>,
    pub container_label_generation: Option<u16>,
    pub observed_docker_status: String,
}

#[derive(Debug, Error)]
pub enum OwnershipError {
    #[error("CAS error: {0}")]
    Cas(#[from] CasError),
    #[error("runtime owner proof invalid: {0}")]
    Proof(&'static str),
    #[error("runtime owner generation overflow")]
    GenerationOverflow,
}

#[derive(Debug, Error)]
pub enum RuntimeIdentityError {
    #[error("runtime identity io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime identity host id error: {0}")]
    HostId(#[from] HostIdError),
    #[error("runtime identity id error: {0}")]
    Id(#[from] IdError),
    #[error("runtime identity procfs data invalid: {0}")]
    Procfs(&'static str),
}

#[derive(Clone, Debug)]
pub struct OwnershipGrant<'a> {
    pub slot_id: &'a str,
    pub operation_id: &'a str,
    pub runtime_incarnation_id: &'a str,
    pub docker_status: &'a str,
    pub fencing_token: &'a str,
    pub owner: Writer,
    pub generation: u16,
    pub now_ms: u64,
    pub event_id: String,
}

pub fn derive_runtime_owner_id(
    slot_id: &str,
    operation_id: &str,
    generation: u16,
) -> Result<String, OwnershipError> {
    if validate_slot_id(slot_id).is_err()
        || crate::contracts::ids::validate_operation_id(operation_id).is_err()
        || validate_generation(generation).is_err()
    {
        return Err(OwnershipError::Proof("runtime owner identity"));
    }
    Ok(derived_id(
        "owner_",
        &serde_json::json!({
            "slotId": slot_id,
            "operationId": operation_id,
            "generation": generation
        }),
    )?)
}

pub fn grant_ownership(input: OwnershipGrant<'_>) -> Result<RuntimeOwnerRecord, OwnershipError> {
    validate_grant(&input)?;
    let id = derive_runtime_owner_id(input.slot_id, input.operation_id, input.generation)?;
    let cas = grant(GrantInput {
        id,
        kind: CasKind::RuntimeOwner,
        subject_id: input.slot_id.to_string(),
        owner: input.owner,
        generation: input.generation,
        fencing_token_sha256: Some(fencing_hash(input.fencing_token)),
        now_ms: input.now_ms,
        event_id: input.event_id,
    })?;
    Ok(RuntimeOwnerRecord {
        cas,
        runtime_incarnation_id: input.runtime_incarnation_id.to_string(),
        docker_status: input.docker_status.to_string(),
    })
}

pub fn adopt_ownership(
    input: OwnershipGrant<'_>,
    proof: &AdoptionProof,
) -> Result<RuntimeOwnerRecord, OwnershipError> {
    validate_adoption_proof(proof)?;
    let record = grant_ownership(input)?;
    let labels_match = match (
        proof.container_label_owner_id.as_deref(),
        proof.container_label_generation,
    ) {
        (None, None) => true,
        (Some(owner_id), Some(generation)) => {
            owner_id == record.cas.id && generation == record.cas.generation
        }
        _ => false,
    };
    if !labels_match || proof.observed_docker_status != record.docker_status {
        return Err(OwnershipError::Proof("AdoptionProof identity"));
    }
    Ok(record)
}

pub fn takeover(
    prior: &RuntimeOwnerRecord,
    proof: &DeadOwnerProof,
    release_id: &str,
    new_owner_writer: Writer,
    takeover_event_id: String,
) -> Result<(RuntimeOwnerRecord, RuntimeOwnerRecord), OwnershipError> {
    validate_dead_owner(prior, proof)?;
    let new_generation = prior
        .cas
        .generation
        .checked_add(1)
        .ok_or(OwnershipError::GenerationOverflow)?;
    let new_owner_id = derived_id(
        "owner_",
        &serde_json::json!({
            "releaseId": release_id,
            "slotId": prior.cas.subject_id,
            "priorOwnerId": prior.cas.id,
            "newGeneration": new_generation
        }),
    )?;
    let mut retired = prior.clone();
    retired.cas.status = "released".to_string();
    retired.cas.released_at_ms = Some(proof.proven_at_ms);
    retired.cas.release_event_id = Some(takeover_event_id.clone());
    retired.cas.last_event_id = takeover_event_id.clone();
    let new_cas = grant(GrantInput {
        id: new_owner_id,
        kind: CasKind::RuntimeOwner,
        subject_id: prior.cas.subject_id.clone(),
        owner: new_owner_writer,
        generation: new_generation,
        fencing_token_sha256: None,
        now_ms: proof.proven_at_ms,
        event_id: takeover_event_id,
    })?;
    let replacement = RuntimeOwnerRecord {
        cas: new_cas,
        runtime_incarnation_id: prior.runtime_incarnation_id.clone(),
        docker_status: prior.docker_status.clone(),
    };
    Ok((retired, replacement))
}

pub fn current_owner_can_stop(
    record: &RuntimeOwnerRecord,
    generation: u16,
    fencing_token: &str,
    now_ms: u64,
) -> bool {
    crate::claims::renewal::verify_active(&record.cas, generation, Some(fencing_token), now_ms)
        .is_ok()
}

pub fn validate_dead_owner(
    prior: &RuntimeOwnerRecord,
    proof: &DeadOwnerProof,
) -> Result<(), OwnershipError> {
    let labels_do_not_prove_other_live_owner = prior.docker_status != "running"
        || proof.container_label_owner_id.is_none()
        || (proof.container_label_owner_id.as_deref() == Some(prior.cas.id.as_str())
            && proof.container_label_generation == Some(prior.cas.generation));
    let label_pair_valid = match (
        proof.container_label_owner_id.as_deref(),
        proof.container_label_generation,
    ) {
        (None, None) => true,
        (Some(owner_id), Some(generation)) => validate_owner_id(owner_id).is_ok() && generation > 0,
        _ => false,
    };
    let valid = prior.cas.status == "active"
        && proof.prior_owner_id == prior.cas.id
        && proof.prior_generation == prior.cas.generation
        && validate_owner_id(&proof.prior_owner_id).is_ok()
        && validate_generation(proof.prior_generation).is_ok()
        && proof.expired_at_ms == prior.cas.expires_at_ms
        && validate_timestamp_ms(proof.expired_at_ms).is_ok()
        && validate_timestamp_ms(proof.grace_satisfied_at_ms).is_ok()
        && validate_timestamp_ms(proof.proven_at_ms).is_ok()
        && proof.grace_satisfied_at_ms >= proof.expired_at_ms.saturating_add(TAKEOVER_GRACE_MS)
        && proof.proven_at_ms >= proof.grace_satisfied_at_ms
        && proof.process_absent
        && proof.lease_inactive
        && proof.claim_inactive
        && label_pair_valid
        && labels_do_not_prove_other_live_owner
        && (1..=8).contains(&proof.evidence_refs.len())
        && proof
            .evidence_refs
            .iter()
            .all(|item| item.validate().is_ok());
    valid
        .then_some(())
        .ok_or(OwnershipError::Proof("DeadOwnerProof"))
}

fn validate_grant(input: &OwnershipGrant<'_>) -> Result<(), OwnershipError> {
    let valid = validate_slot_id(input.slot_id).is_ok()
        && validate_runtime_incarnation_id(input.runtime_incarnation_id).is_ok()
        && validate_generation(input.generation).is_ok()
        && validate_timestamp_ms(input.now_ms).is_ok()
        && matches!(
            input.docker_status,
            "running" | "exited" | "missing" | "starting" | "stopping" | "unknown"
        )
        && !input.fencing_token.is_empty();
    valid
        .then_some(())
        .ok_or(OwnershipError::Proof("OwnershipGrant"))
}

fn validate_adoption_proof(proof: &AdoptionProof) -> Result<(), OwnershipError> {
    let labels = match (
        proof.container_label_owner_id.as_deref(),
        proof.container_label_generation,
    ) {
        (None, None) => true,
        (Some(owner), Some(generation)) => {
            validate_owner_id(owner).is_ok() && validate_generation(generation).is_ok()
        }
        _ => false,
    };
    (labels
        && matches!(
            proof.observed_docker_status.as_str(),
            "running" | "exited" | "missing" | "starting" | "stopping" | "unknown"
        ))
    .then_some(())
    .ok_or(OwnershipError::Proof("AdoptionProof"))
}

pub fn generic(record: &RuntimeOwnerRecord) -> &CasRecord {
    &record.cas
}

pub fn current_writer(host_id_seed_path: &Path) -> Result<Writer, RuntimeIdentityError> {
    let host_id = load_or_create_host_id(host_id_seed_path)?;
    let process_id = std::process::id();
    let process_start_ms = linux_process_start_ms(process_id)?;
    let writer_id = derive_writer_id(&host_id, process_id, process_start_ms)?;
    Ok(Writer {
        host_id,
        process_id,
        process_start_ms,
        writer_id,
    })
}

pub fn process_absent(
    recorded: &Writer,
    local_host_id: &str,
) -> Result<bool, RuntimeIdentityError> {
    if recorded.host_id != local_host_id {
        return Ok(false);
    }
    match linux_process_start_ms(recorded.process_id) {
        Ok(observed) => Ok(observed != recorded.process_start_ms),
        Err(RuntimeIdentityError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub fn process_absent_from_observation(
    recorded: &Writer,
    local_host_id: &str,
    observed_process_start_ms: Option<u64>,
) -> bool {
    recorded.host_id == local_host_id
        && observed_process_start_ms != Some(recorded.process_start_ms)
}

pub fn linux_process_start_ms(process_id: u32) -> Result<u64, RuntimeIdentityError> {
    let proc_stat = fs::read_to_string(PathBuf::from(format!("/proc/{process_id}/stat")))?;
    let system_stat = fs::read_to_string("/proc/stat")?;
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if clock_ticks <= 0 {
        return Err(RuntimeIdentityError::Procfs("_SC_CLK_TCK"));
    }
    process_start_ms_from_proc(&proc_stat, &system_stat, clock_ticks as u64)
}

pub fn process_start_ms_from_proc(
    process_stat: &str,
    system_stat: &str,
    clock_ticks_per_second: u64,
) -> Result<u64, RuntimeIdentityError> {
    if clock_ticks_per_second == 0 {
        return Err(RuntimeIdentityError::Procfs("clock ticks"));
    }
    let closing_paren = process_stat
        .rfind(')')
        .ok_or(RuntimeIdentityError::Procfs("process stat comm"))?;
    let start_ticks = process_stat[closing_paren + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or(RuntimeIdentityError::Procfs("process stat starttime"))?
        .parse::<u64>()
        .map_err(|_| RuntimeIdentityError::Procfs("process stat starttime"))?;
    let boot_seconds = system_stat
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .ok_or(RuntimeIdentityError::Procfs("system stat btime"))?
        .trim()
        .parse::<u64>()
        .map_err(|_| RuntimeIdentityError::Procfs("system stat btime"))?;
    let boot_ms = boot_seconds
        .checked_mul(1_000)
        .ok_or(RuntimeIdentityError::Procfs("btime overflow"))?;
    let tick_ms = start_ticks
        .checked_mul(1_000)
        .ok_or(RuntimeIdentityError::Procfs("starttime overflow"))?
        / clock_ticks_per_second;
    let result = boot_ms
        .checked_add(tick_ms)
        .ok_or(RuntimeIdentityError::Procfs("process start overflow"))?;
    validate_timestamp_ms(result)?;
    Ok(result)
}
