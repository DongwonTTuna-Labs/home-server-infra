use crate::contracts::cli::CommandOutcome;

pub(super) fn with_allocation_identity(
    mut outcome: CommandOutcome,
    request_id: String,
    run_id: String,
    slot_id: Option<String>,
    cohort: Option<String>,
) -> CommandOutcome {
    outcome.envelope.request_id = Some(request_id);
    outcome.envelope.run_id = Some(run_id);
    outcome.envelope.slot_id = slot_id;
    outcome.envelope.cohort = cohort;
    outcome
}
