pub mod events;
pub mod receipts;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use thiserror::Error;

use crate::contracts::cli::result_spec;
use crate::contracts::events::EventType;
use crate::contracts::provider::ProviderOperation;

use self::events::{parse_r13_event_sequence, EventTokenError};

const CROSSWALK_HEADER: &str = "normalizedLeafId\tr13ResponseDiscriminant\trequiredProofOrReceipt\tr13EventSequence\tlifecycleResultKind\texit\tfailClosedResultKind";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponsePolarity {
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseDiscriminant {
    pub operation: ProviderOperation,
    pub polarity: ResponsePolarity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredProofOrReceipt {
    SendPostClick,
    SendReconciledTurnStart,
    PollReceipt,
    PlaywrightDownloadReceipt,
    ZeroControlProof,
    SessionEcho,
    UploadProof,
    ModelProof,
    RootBindingCandidate,
    StatusProbe,
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrosswalkRow {
    pub normalized_leaf_id: String,
    pub response: ResponseDiscriminant,
    pub required: RequiredProofOrReceipt,
    pub events: Vec<EventType>,
    pub lifecycle_result_kind: String,
    pub exit: u8,
    pub fail_closed_result_kind: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CrosswalkCatalog {
    rows: BTreeMap<String, CrosswalkRow>,
}

#[derive(Debug, Error)]
pub enum NormalizationError {
    #[error("normalization catalog io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("normalization catalog bytes are not canonical TSV: {0}")]
    CatalogBytes(&'static str),
    #[error("crosswalk bytes are not canonical TSV: {0}")]
    CrosswalkBytes(&'static str),
    #[error("crosswalk header differs from the R23 seven-column header")]
    Header,
    #[error("crosswalk row has an invalid field count")]
    FieldCount,
    #[error("crosswalk normalized leaf set differs from the normalized catalog")]
    LeafParity,
    #[error("duplicate normalized leaf id: {0}")]
    DuplicateLeaf(String),
    #[error("crosswalk rows are not LC_ALL=C ordered by normalized leaf id")]
    LeafOrder,
    #[error("invalid R13 response discriminant: {0}")]
    InvalidDiscriminant(String),
    #[error("invalid required proof or receipt: {0}")]
    InvalidProof(String),
    #[error("invalid R13 event sequence: {0}")]
    InvalidEventSequence(#[from] EventTokenError),
    #[error("lifecycle result kind or exit is not in the closed matrix: {0}")]
    ResultExitMismatch(String),
    #[error("invalid fail-closed lifecycle result kind: {0}")]
    InvalidFailClosed(String),
    #[error("normalized leaf is absent from the crosswalk: {0}")]
    CrosswalkMissing(String),
}

impl CrosswalkCatalog {
    pub fn lookup(&self, normalized_leaf_id: &str) -> Result<&CrosswalkRow, NormalizationError> {
        self.rows
            .get(normalized_leaf_id)
            .ok_or_else(|| NormalizationError::CrosswalkMissing(normalized_leaf_id.to_string()))
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

pub fn load_crosswalk(
    normalized_catalog: &Path,
    crosswalk: &Path,
) -> Result<CrosswalkCatalog, NormalizationError> {
    let normalized_bytes = fs::read(normalized_catalog)?;
    validate_tsv_bytes(&normalized_bytes, false)?;
    let normalized_text = std::str::from_utf8(&normalized_bytes)
        .map_err(|_| NormalizationError::CatalogBytes("UTF-8"))?;
    let expected_leaves = normalized_leaf_ids(normalized_text)?;

    let crosswalk_bytes = fs::read(crosswalk)?;
    validate_tsv_bytes(&crosswalk_bytes, true)?;
    let crosswalk_text = std::str::from_utf8(&crosswalk_bytes)
        .map_err(|_| NormalizationError::CrosswalkBytes("UTF-8"))?;
    let mut lines = crosswalk_text.lines();
    if lines.next() != Some(CROSSWALK_HEADER) {
        return Err(NormalizationError::Header);
    }

    let mut rows = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 7 {
            return Err(NormalizationError::FieldCount);
        }
        let leaf = fields[0];
        if leaf.is_empty() {
            return Err(NormalizationError::FieldCount);
        }
        if previous.is_some_and(|prior| prior.as_bytes() >= leaf.as_bytes()) {
            return Err(NormalizationError::LeafOrder);
        }
        previous = Some(leaf);
        let row = parse_row(&fields)?;
        if rows.insert(leaf.to_string(), row).is_some() {
            return Err(NormalizationError::DuplicateLeaf(leaf.to_string()));
        }
    }
    if rows.keys().cloned().collect::<BTreeSet<_>>() != expected_leaves {
        return Err(NormalizationError::LeafParity);
    }
    Ok(CrosswalkCatalog { rows })
}

fn parse_row(fields: &[&str]) -> Result<CrosswalkRow, NormalizationError> {
    let response = parse_discriminant(fields[1])?;
    let required = parse_required(fields[2])?;
    if !fields[3].bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b',' | b'|' | b'-')
    }) {
        return Err(NormalizationError::CrosswalkBytes("event list cell"));
    }
    let events = parse_r13_event_sequence(fields[3])?;
    let exit = parse_exit(fields[5])?;
    let command = command_for_result(fields[4]);
    let spec = result_spec(command, fields[4])
        .filter(|spec| spec.exit_code == exit)
        .ok_or_else(|| NormalizationError::ResultExitMismatch(fields[4].to_string()))?;
    let fail_command = command_for_result(fields[6]);
    let fail_spec = result_spec(fail_command, fields[6])
        .filter(|candidate| !candidate.ok && candidate.exit_code == 70 && candidate.terminal)
        .ok_or_else(|| NormalizationError::InvalidFailClosed(fields[6].to_string()))?;
    let _ = (spec, fail_spec);
    Ok(CrosswalkRow {
        normalized_leaf_id: fields[0].to_string(),
        response,
        required,
        events,
        lifecycle_result_kind: fields[4].to_string(),
        exit,
        fail_closed_result_kind: fields[6].to_string(),
    })
}

fn validate_tsv_bytes(bytes: &[u8], crosswalk: bool) -> Result<(), NormalizationError> {
    let invalid = bytes.is_empty()
        || bytes.starts_with(&[0xef, 0xbb, 0xbf])
        || !bytes.ends_with(b"\n")
        || bytes.contains(&b'\r')
        || bytes.contains(&0)
        || bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .any(|line| matches!(line.last(), Some(b' ' | b'\t')));
    if invalid {
        return if crosswalk {
            Err(NormalizationError::CrosswalkBytes("serialization"))
        } else {
            Err(NormalizationError::CatalogBytes("serialization"))
        };
    }
    Ok(())
}

fn normalized_leaf_ids(text: &str) -> Result<BTreeSet<String>, NormalizationError> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or(NormalizationError::CatalogBytes("header"))?
        .split('\t')
        .collect::<Vec<_>>();
    let index = header
        .iter()
        .position(|field| *field == "normalized_leaf_id")
        .ok_or(NormalizationError::CatalogBytes("normalized_leaf_id"))?;
    let mut leaves = BTreeSet::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        let leaf = fields
            .get(index)
            .filter(|value| !value.is_empty())
            .ok_or(NormalizationError::CatalogBytes("row"))?;
        if !leaves.insert((*leaf).to_string()) {
            return Err(NormalizationError::DuplicateLeaf((*leaf).to_string()));
        }
    }
    Ok(leaves)
}

fn parse_discriminant(value: &str) -> Result<ResponseDiscriminant, NormalizationError> {
    let (operation, polarity) = value
        .rsplit_once('.')
        .ok_or_else(|| NormalizationError::InvalidDiscriminant(value.to_string()))?;
    let operation = match operation {
        "status" => ProviderOperation::Status,
        "capture.root" => ProviderOperation::CaptureRoot,
        "ensure-model" => ProviderOperation::EnsureModel,
        "upload-only" => ProviderOperation::UploadOnly,
        "clear-upload" => ProviderOperation::ClearUpload,
        "send-click" => ProviderOperation::SendClick,
        "send-reconcile" => ProviderOperation::SendReconcile,
        "session-rebind" => ProviderOperation::SessionRebind,
        "poll" => ProviderOperation::Poll,
        "artifact-discover" => ProviderOperation::ArtifactDiscover,
        "artifact-click-save" => ProviderOperation::ArtifactClickSave,
        _ => return Err(NormalizationError::InvalidDiscriminant(value.to_string())),
    };
    let polarity = match polarity {
        "success" => ResponsePolarity::Success,
        "failure" => ResponsePolarity::Failure,
        _ => return Err(NormalizationError::InvalidDiscriminant(value.to_string())),
    };
    Ok(ResponseDiscriminant {
        operation,
        polarity,
    })
}

fn parse_required(value: &str) -> Result<RequiredProofOrReceipt, NormalizationError> {
    match value {
        "send_receipt.post_click" => Ok(RequiredProofOrReceipt::SendPostClick),
        "send_receipt.reconciled_turn_start" => Ok(RequiredProofOrReceipt::SendReconciledTurnStart),
        "poll_receipt" => Ok(RequiredProofOrReceipt::PollReceipt),
        "playwright_download_receipt" => Ok(RequiredProofOrReceipt::PlaywrightDownloadReceipt),
        "zero_control_proof" => Ok(RequiredProofOrReceipt::ZeroControlProof),
        "session_echo" => Ok(RequiredProofOrReceipt::SessionEcho),
        "upload_proof" => Ok(RequiredProofOrReceipt::UploadProof),
        "model_proof" => Ok(RequiredProofOrReceipt::ModelProof),
        "root_binding_candidate" => Ok(RequiredProofOrReceipt::RootBindingCandidate),
        "status_probe" => Ok(RequiredProofOrReceipt::StatusProbe),
        "none" => Ok(RequiredProofOrReceipt::None),
        _ => Err(NormalizationError::InvalidProof(value.to_string())),
    }
}

fn parse_exit(value: &str) -> Result<u8, NormalizationError> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(NormalizationError::ResultExitMismatch(value.to_string()));
    }
    value
        .parse::<u8>()
        .map_err(|_| NormalizationError::ResultExitMismatch(value.to_string()))
}

fn command_for_result(result_kind: &str) -> &str {
    match result_kind.split_once('.').map(|(prefix, _)| prefix) {
        Some("state_rebuild") => "state-rebuild",
        Some(command) => command,
        None => "",
    }
}
