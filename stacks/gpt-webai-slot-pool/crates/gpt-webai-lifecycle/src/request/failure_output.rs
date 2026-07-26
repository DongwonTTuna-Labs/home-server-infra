use crate::slots::AllocationDecision;

use super::input::RequestRunInput;
use super::output::{failed_output, RequestRunOutput};
use super::runtime::RuntimeStart;

pub(crate) fn failed_after_runtime_start(
    input: &RequestRunInput,
    decision: &AllocationDecision,
    reason: &str,
    message: String,
    runtime_start: &RuntimeStart,
) -> RequestRunOutput {
    failed_output(input, Some(decision), reason, message)
        .with_runtime_started(runtime_start.runtime_started)
        .with_runtime_owned(runtime_start.runtime_owned)
}
