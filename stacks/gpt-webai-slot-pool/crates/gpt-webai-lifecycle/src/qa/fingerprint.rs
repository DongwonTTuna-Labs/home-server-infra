use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::ids::{h256, validate_byte_count, validate_h256, validate_safe_rel_path};
use crate::journal::canonical::canonical_bytes;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceContentEntry {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum FingerprintError {
    #[error("invalid source fingerprint entry: {0}")]
    Invalid(&'static str),
    #[error("source fingerprint encoding failed")]
    Encoding,
}

pub fn fingerprint_entries(entries: &[SourceContentEntry]) -> Result<String, FingerprintError> {
    let mut prior: Option<&str> = None;
    let mut unique = BTreeSet::new();
    for entry in entries {
        validate_safe_rel_path(&entry.path).map_err(|_| FingerprintError::Invalid("path"))?;
        validate_h256(&entry.sha256).map_err(|_| FingerprintError::Invalid("sha256"))?;
        validate_byte_count(entry.size_bytes)
            .map_err(|_| FingerprintError::Invalid("sizeBytes"))?;
        if excluded_runtime_path(&entry.path)
            || prior.is_some_and(|value| value >= entry.path.as_str())
            || !unique.insert(entry.path.as_str())
        {
            return Err(FingerprintError::Invalid("scope/order"));
        }
        prior = Some(&entry.path);
    }
    canonical_bytes(&entries)
        .map(h256)
        .map_err(|_| FingerprintError::Encoding)
}

pub fn excluded_runtime_path(path: &str) -> bool {
    path == ".git"
        || path.starts_with(".git/")
        || path == ".omo/evidence"
        || path.starts_with(".omo/evidence/")
        || path == "target"
        || path.starts_with("target/")
        || path.contains("/target/")
        || path == "node_modules"
        || path.starts_with("node_modules/")
        || path.contains("/node_modules/")
}
