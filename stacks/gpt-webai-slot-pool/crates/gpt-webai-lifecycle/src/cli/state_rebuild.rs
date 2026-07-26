use crate::config::SupervisorConfig;
use crate::contracts::cli::CommandOutcome;
use crate::contracts::ids::derive_session_binding_id;
use crate::errors::LifecycleError;
use crate::journal::{
    EventStore, PersistedSessionSeed, RebuildCheckError, RebuildHeadObservation, SnapshotInspection,
};
use crate::sessions::read_all_session_records;
use std::collections::BTreeMap;

use super::{acquire_command_guard, emit_command_outcome, new_operation_id};

pub fn run(args: &[String]) -> Result<u8, LifecycleError> {
    if args != ["--json", "--check-only"] {
        return Err(LifecycleError::Usage(
            "state-rebuild requires exactly --json --check-only".to_string(),
        ));
    }
    let operation_id = new_operation_id("state-rebuild")?;
    let config = SupervisorConfig::from_env();
    let _guard = match acquire_command_guard(&config.state_root, "state-rebuild", &operation_id)? {
        Ok(guard) => guard,
        Err(exit_code) => return Ok(exit_code),
    };
    let seeds = match persisted_session_seeds(&config) {
        Ok(seeds) => seeds,
        Err(message) => {
            return emit_command_outcome(CommandOutcome::select(
                "state-rebuild",
                operation_id,
                "state_rebuild.event_invalid",
                message,
                Some("state_rebuild.event_invalid".to_string()),
            ));
        }
    };
    let outcome = match EventStore::new(&config.state_root).inspect_rebuild_check_only(&seeds) {
        Ok(inspection) => match (&inspection.head, &inspection.snapshot) {
            (RebuildHeadObservation::Stale(head), SnapshotInspection::Ignored(snapshot)) => {
                CommandOutcome::select(
                    "state-rebuild",
                    operation_id,
                    "state_rebuild.head_stale",
                    format!("{head}; subordinate snapshot ignored: {snapshot}"),
                    None,
                )
            }
            (RebuildHeadObservation::Stale(head), _) => CommandOutcome::select(
                "state-rebuild",
                operation_id,
                "state_rebuild.head_stale",
                head.clone(),
                None,
            ),
            (RebuildHeadObservation::Match, SnapshotInspection::Ignored(snapshot)) => {
                CommandOutcome::select(
                    "state-rebuild",
                    operation_id,
                    "state_rebuild.snapshot_ignored",
                    snapshot.clone(),
                    None,
                )
            }
            (RebuildHeadObservation::Match, SnapshotInspection::Absent) => CommandOutcome::select(
                "state-rebuild",
                operation_id,
                "state_rebuild.match",
                "event replay, HEAD, and projections match; no snapshot is referenced",
                None,
            ),
            (RebuildHeadObservation::Match, SnapshotInspection::Trusted) => CommandOutcome::select(
                "state-rebuild",
                operation_id,
                "state_rebuild.match",
                "event replay, HEAD, projections, and trusted snapshot match",
                None,
            ),
        },
        Err(error) => failure_outcome(operation_id, error),
    };
    emit_command_outcome(outcome)
}

fn failure_outcome(
    operation_id: String,
    error: RebuildCheckError,
) -> Result<CommandOutcome, crate::contracts::cli::CommandOutcomeError> {
    let result_kind = match error {
        RebuildCheckError::EventInvalid(_) => "state_rebuild.event_invalid",
        RebuildCheckError::TransitionInvalid(_) => "state_rebuild.transition_invalid",
        RebuildCheckError::DigestMismatch(_) => "state_rebuild.digest_mismatch",
    };
    CommandOutcome::select(
        "state-rebuild",
        operation_id,
        result_kind,
        error.to_string(),
        Some(result_kind.to_string()),
    )
}

fn persisted_session_seeds(
    config: &SupervisorConfig,
) -> Result<BTreeMap<String, PersistedSessionSeed>, String> {
    let records =
        read_all_session_records(&config.state_root).map_err(|error| error.to_string())?;
    records
        .into_iter()
        .map(|record| {
            let binding =
                derive_session_binding_id(&record.session_id, &record.slot_id, &record.cohort)
                    .map_err(|error| error.to_string())?;
            Ok((
                record.session_id.clone(),
                PersistedSessionSeed {
                    session_id: record.session_id,
                    session_binding_id: Some(binding),
                    conversation_url: record.conversation_url,
                    slot_id: record.slot_id,
                    cohort: record.cohort,
                    page_binding_generation: Some(record.page_binding_generation),
                },
            ))
        })
        .collect()
}
