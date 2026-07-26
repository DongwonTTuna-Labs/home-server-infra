use thiserror::Error;

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("{0}")]
    Usage(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl LifecycleError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Io(_) | Self::Json(_) => 70,
        }
    }
}
