use crate::locks::{self, LockError};
use crate::sessions::{self, SessionRecord, SessionRecordError};

use super::error::ResumeError;
use super::ResumeInput;

pub(super) fn clear_stale_resume_lease(
    input: &ResumeInput,
    record: &SessionRecord,
) -> Result<(), ResumeError> {
    match locks::read_slot_lease(&input.config.state_root, &record.slot_id) {
        Ok(lease) if !locks::lease_is_stale(&lease) => Err(ResumeError::ActiveLease),
        Ok(_) => match locks::release_stale_slot_lease(&input.config.state_root, &record.slot_id) {
            Ok(_) | Err(LockError::Missing(_)) => Ok(()),
            Err(LockError::Busy(_)) => Err(ResumeError::ActiveLease),
            Err(error) => Err(ResumeError::LockRelease(error)),
        },
        Err(LockError::Missing(_)) => Ok(()),
        Err(error) => Err(ResumeError::LockRead(error)),
    }
}

pub(super) fn mark_resume_released(
    input: &ResumeInput,
    record: &SessionRecord,
) -> Result<(), SessionRecordError> {
    let released = sessions::mark_session_released(record.clone(), Some("answer.done".to_string()));
    sessions::update_session_record(&input.config.state_root, &released)
}
