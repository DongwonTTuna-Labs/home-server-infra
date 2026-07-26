use crate::config::SupervisorConfig;
use crate::sessions::{read_session_record, SessionRecordError};

use super::args::ReleaseArgs;

pub(super) struct ReleaseTarget {
    pub slot_id: String,
    pub session_id: Option<String>,
}

pub(super) struct TargetError {
    pub session_id: String,
    pub reason: &'static str,
    pub message: String,
}

pub(super) fn resolve(
    config: &SupervisorConfig,
    args: &ReleaseArgs,
) -> Result<ReleaseTarget, TargetError> {
    if let Some(slot_id) = &args.slot_id {
        return Ok(ReleaseTarget {
            slot_id: slot_id.clone(),
            session_id: None,
        });
    }
    let session_id = args
        .session_id
        .clone()
        .expect("release target validation requires slot or session");
    match read_session_record(&config.state_root, &session_id) {
        Ok(record) => Ok(ReleaseTarget {
            slot_id: record.slot_id,
            session_id: Some(session_id),
        }),
        Err(error) => Err(TargetError {
            reason: session_error_reason(&error),
            session_id,
            message: error.to_string(),
        }),
    }
}

fn session_error_reason(error: &SessionRecordError) -> &'static str {
    match error {
        SessionRecordError::Missing(_) => "session.record_missing",
        SessionRecordError::Collision(_)
        | SessionRecordError::Invalid(_)
        | SessionRecordError::InvalidConversationUrl(_)
        | SessionRecordError::Json(_) => "session.record_invalid",
        SessionRecordError::Io(_) => "session.record_read_failed",
    }
}
