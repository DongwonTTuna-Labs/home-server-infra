use crate::request::input::RequestRunInput;
use crate::sessions::{self, SessionRecord};

pub(crate) fn mark_released(
    input: &RequestRunInput,
    session: SessionRecord,
    reason: Option<String>,
) {
    let released = sessions::mark_session_released(session, reason);
    let _ = sessions::update_session_record(&input.config.state_root, &released);
}

pub(crate) fn mark_release_failed_by_id(input: &RequestRunInput, session_id: &str, reason: &str) {
    let Ok(mut session) = sessions::read_session_record(&input.config.state_root, session_id)
    else {
        return;
    };
    let _ = reason;
    session.updated_at_ms = session.updated_at_ms.saturating_add(1);
    let _ = sessions::update_session_record(&input.config.state_root, &session);
}
