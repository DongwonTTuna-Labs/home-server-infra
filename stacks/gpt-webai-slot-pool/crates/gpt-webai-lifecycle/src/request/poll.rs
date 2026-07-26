use std::io;

use serde_json::Value;

use crate::artifact_objects::{write_provider_poll_artifact_objects, ArtifactObjectContext};
use crate::provider_runner::ProviderCommand;
use crate::sessions::SessionRecord;

use super::input::RequestRunInput;

pub(crate) fn write_nonterminal_poll_artifacts(
    input: &RequestRunInput,
    provider_command: &ProviderCommand,
    session: &SessionRecord,
    poll_value: &Value,
) -> io::Result<()> {
    let (request_id, run_id) = session.request_binding().map_err(io::Error::other)?;
    write_provider_poll_artifact_objects(
        ArtifactObjectContext {
            config: &input.config,
            path_mode: &provider_command.path_mode,
            request_id,
            run_id,
            session_id: &session.session_id,
            conversation_url: &session.conversation_url,
            slot_id: &session.slot_id,
            account_group: &session.cohort,
        },
        poll_value,
    )
}
