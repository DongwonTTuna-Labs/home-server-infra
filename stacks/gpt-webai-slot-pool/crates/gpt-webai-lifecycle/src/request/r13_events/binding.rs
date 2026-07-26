use serde_json::json;

use crate::contracts::browser::{PageBindingEcho, RootBindingCandidate};
use crate::contracts::events::EventEnvelope;
use crate::contracts::ids::{derive_page_binding_id, h256};
use crate::journal::canonical::canonical_bytes;

use super::send::SendEventError;

pub fn build_page_binding(
    root: &RootBindingCandidate,
    slot_id: &str,
    cohort: &str,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
) -> Result<PageBindingEcho, SendEventError> {
    root.validate()
        .map_err(|_| SendEventError::Contract("rootBindingCandidate"))?;
    let root_binding_hash = root_binding_hash(root)?;
    let page = PageBindingEcho {
        binding_id: derive_page_binding_id(&root.page_incarnation_id, &root_binding_hash)?,
        binding_generation: 1,
        slot_id: slot_id.to_string(),
        cohort: cohort.to_string(),
        lease_id: lease.aggregate.id.clone(),
        lease_generation: 1,
        runtime_owner_id: owner.aggregate.id.clone(),
        runtime_owner_generation: owner.payload["ownerGeneration"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(SendEventError::Contract("ownerGeneration"))?,
        runtime_incarnation_id: owner.payload["runtimeIncarnationId"]
            .as_str()
            .ok_or(SendEventError::Contract("runtimeIncarnationId"))?
            .to_string(),
        browser_context_id: root.browser_context_id.clone(),
        target_id: root.target_id.clone(),
        page_incarnation_id: root.page_incarnation_id.clone(),
        root_binding_hash,
        dom_mutation_generation: root.dom_mutation_generation,
    };
    page.validate()
        .map_err(|_| SendEventError::Contract("pageBinding"))?;
    Ok(page)
}

fn root_binding_hash(root: &RootBindingCandidate) -> Result<String, serde_json::Error> {
    Ok(h256(canonical_bytes(&json!([
        root.conversation_root_id,
        root.composer_root_id,
        root.model_control.control_id,
        root.effort_control.control_id,
        root.normalized_url,
        root.dom_mutation_generation
    ]))?))
}
