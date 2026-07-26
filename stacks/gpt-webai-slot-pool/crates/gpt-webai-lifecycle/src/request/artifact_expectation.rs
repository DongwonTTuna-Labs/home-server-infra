use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactExpectation {
    None,
    #[default]
    Optional,
    Required,
    Claimed,
}

impl ArtifactExpectation {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "optional" => Some(Self::Optional),
            "required" => Some(Self::Required),
            "claimed" => Some(Self::Claimed),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Optional => "optional",
            Self::Required => "required",
            Self::Claimed => "claimed",
        }
    }

    pub fn requires_download_controls(self) -> bool {
        matches!(self, Self::Required | Self::Claimed)
    }

    pub fn from_prompt(prompt: &str) -> Self {
        let value = prompt.to_ascii_lowercase();
        let artifact_words = [
            "downloadable artifact",
            "downloadable archive",
            "downloadable zip",
            "downloadable tar",
            "source-tree artifact",
            "coding artifact",
            "artifact_ready",
            "artifact ready",
            "zip or tar artifact",
            "tar.gz",
            ".zip",
        ];
        if artifact_words.iter().any(|needle| value.contains(needle)) {
            Self::Required
        } else {
            Self::Optional
        }
    }
}
