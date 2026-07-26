use std::collections::BTreeMap;
use std::env;

use serde_json::Value;
use thiserror::Error;

use crate::claims::CasError;
use crate::config::{resolve_host_id_seed_path, SupervisorConfig};
use crate::contracts::events::{
    Aggregate, AggregateKind, EventEnvelope, EventError, EventType, Writer,
};
use crate::contracts::ids::derive_session_binding_id;
use crate::journal::replay::topological;
use crate::journal::{
    EventStore, EventStoreError, HeadStore, MutationGuard, PersistedSessionSeed, ReducedProjection,
};
use crate::runtime::ownership::{current_writer, RuntimeIdentityError};
use crate::sessions::{read_all_session_records, SessionRecordError};

#[derive(Debug, Error)]
pub enum SessionJournalError {
    #[error("session journal event invalid: {0}")]
    Event(#[from] EventError),
    #[error("session journal commit failed: {0}")]
    Store(#[from] EventStoreError),
    #[error("session journal lock failed: {0}")]
    Head(#[from] crate::journal::head::HeadError),
    #[error("session journal persisted-session seed failed: {0}")]
    Session(#[from] SessionRecordError),
    #[error("session journal identity failed: {0}")]
    Identity(#[from] RuntimeIdentityError),
    #[error("session journal binding derivation failed: {0}")]
    Binding(#[from] crate::contracts::ids::IdError),
    #[error("session journal CAS identifier derivation failed: {0}")]
    Cas(#[from] CasError),
    #[error("host-id seed path is unavailable")]
    HostSeedUnavailable,
}

pub struct SessionJournal {
    operation_id: String,
    request_id: Option<String>,
    run_id: Option<String>,
    seeds: BTreeMap<String, PersistedSessionSeed>,
    state_root: std::path::PathBuf,
    writer: Writer,
    event_ids: Vec<String>,
}

pub struct NewEvent {
    pub aggregate_kind: AggregateKind,
    pub aggregate_id: String,
    pub event_type: EventType,
    pub payload: Value,
    pub predecessor_event_id: Option<String>,
    pub source_event_ids: Vec<String>,
    pub created_at_ms: u64,
}

impl SessionJournal {
    pub fn open(
        config: &SupervisorConfig,
        operation_id: impl Into<String>,
        request_id: Option<String>,
        run_id: Option<String>,
    ) -> Result<Self, SessionJournalError> {
        let host_seed = resolve_host_id_seed_path(
            env::var_os("XDG_STATE_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        )
        .ok_or(SessionJournalError::HostSeedUnavailable)?;
        Ok(Self {
            operation_id: operation_id.into(),
            request_id,
            run_id,
            seeds: persisted_session_seeds(config)?,
            state_root: config.state_root.clone(),
            writer: current_writer(&host_seed)?,
            event_ids: Vec::new(),
        })
    }

    pub fn replay(&self) -> Result<ReducedProjection, SessionJournalError> {
        Ok(EventStore::new(&self.state_root).replay(&self.seeds)?)
    }

    pub fn append(&mut self, input: NewEvent) -> Result<EventEnvelope, SessionJournalError> {
        let head = HeadStore::new(&self.state_root);
        let guard = head.acquire_mutation()?;
        let event = self.append_with_guard(&guard, input)?;
        drop(guard);
        hit_event_failpoint(event.event_type);
        Ok(event)
    }

    pub fn append_with_guard(
        &mut self,
        guard: &MutationGuard,
        input: NewEvent,
    ) -> Result<EventEnvelope, SessionJournalError> {
        let event = EventEnvelope::create(
            Aggregate {
                id: input.aggregate_id,
                kind: input.aggregate_kind,
            },
            input.created_at_ms,
            input.event_type,
            self.operation_id.clone(),
            input.payload,
            input.predecessor_event_id,
            self.request_id.clone(),
            self.run_id.clone(),
            input.source_event_ids,
            self.writer.clone(),
        )?;
        EventStore::new(&self.state_root).append_transaction_with_seeds(
            guard,
            std::slice::from_ref(&event),
            &self.seeds,
        )?;
        self.event_ids.push(event.event_id.clone());
        Ok(event)
    }

    pub fn event_ids(&self) -> &[String] {
        &self.event_ids
    }

    pub fn aggregate_tail_event_id(
        &self,
        aggregate_kind: AggregateKind,
        aggregate_id: &str,
    ) -> Result<Option<String>, SessionJournalError> {
        let events = EventStore::new(&self.state_root).load_all()?;
        let ordered = topological(&events).map_err(EventStoreError::from)?;
        Ok(ordered
            .into_iter()
            .rev()
            .find(|event| {
                event.aggregate.kind == aggregate_kind && event.aggregate.id == aggregate_id
            })
            .map(|event| event.event_id))
    }

    pub fn writer(&self) -> &Writer {
        &self.writer
    }
}

fn hit_event_failpoint(event_type: EventType) {
    use EventType::{
        AnswerTerminal, ReleaseEvidencePreserved, RequestClaimReleased, RuntimeOwnershipGranted,
        RuntimeOwnershipReleased, RuntimeStopFailed, RuntimeStopSkipped, RuntimeStopped,
        SendClickArmed, SessionOperationClaimReleased, SessionRuntimeOwnershipGranted,
        SlotLeaseReleased, TerminalPersisted, TurnStartConfirmed, UploadCleared,
    };

    match event_type {
        UploadCleared => crate::failpoint::hit("after-uploadcleared"),
        SendClickArmed => crate::failpoint::hit("after-sendclickarmed"),
        TurnStartConfirmed => crate::failpoint::hit("after-turnstartconfirmed"),
        RuntimeOwnershipGranted | SessionRuntimeOwnershipGranted => {
            crate::failpoint::hit("after-session-claim-lease-owner-renewal");
        }
        AnswerTerminal => crate::failpoint::hit("after-answerterminal"),
        TerminalPersisted => crate::failpoint::hit("after-terminalpersisted"),
        ReleaseEvidencePreserved => crate::failpoint::hit("after-evidence-preservation"),
        RuntimeStopped | RuntimeStopFailed | RuntimeStopSkipped => {
            crate::failpoint::hit("after-runtime-stop-before-resource-release");
        }
        SessionOperationClaimReleased
        | RequestClaimReleased
        | SlotLeaseReleased
        | RuntimeOwnershipReleased => {
            crate::failpoint::hit("after-each-exactly-once-release-event");
        }
        _ => {}
    }
}

pub(crate) fn persisted_session_seeds(
    config: &SupervisorConfig,
) -> Result<BTreeMap<String, PersistedSessionSeed>, SessionJournalError> {
    read_all_session_records(&config.state_root)?
        .into_iter()
        .map(|record| {
            let binding =
                derive_session_binding_id(&record.session_id, &record.slot_id, &record.cohort)?;
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
