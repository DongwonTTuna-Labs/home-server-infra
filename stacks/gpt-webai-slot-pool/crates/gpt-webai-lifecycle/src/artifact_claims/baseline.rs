use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{
    validate_byte_count, validate_h256, validate_safe_rel_path, validate_timestamp_ms,
};

use super::{valid, ArtifactClaimError};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactBaselineEntry {
    pub rel_path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactBaseline {
    pub directory: String,
    pub entries: Vec<ArtifactBaselineEntry>,
    pub captured_at_ms: u64,
    pub baseline_sha256: String,
}

impl ArtifactBaseline {
    pub fn validate(&self) -> Result<(), ArtifactClaimError> {
        valid(validate_safe_rel_path(&self.directory), "directory")?;
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        valid(validate_h256(&self.baseline_sha256), "baselineSha256")?;
        if self.entries.len() > 128 {
            return Err(ArtifactClaimError::Invalid("baseline entries"));
        }
        let mut previous: Option<&str> = None;
        let mut unique = BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if previous.is_some_and(|value| value >= entry.rel_path.as_str())
                || !unique.insert(entry.rel_path.as_str())
            {
                return Err(ArtifactClaimError::Invalid("baseline ordering"));
            }
            previous = Some(&entry.rel_path);
        }
        Ok(())
    }
}

impl ArtifactBaselineEntry {
    fn validate(&self) -> Result<(), ArtifactClaimError> {
        valid(validate_safe_rel_path(&self.rel_path), "relPath")?;
        valid(validate_byte_count(self.size_bytes), "sizeBytes")?;
        valid(validate_h256(&self.sha256), "sha256")
    }
}
