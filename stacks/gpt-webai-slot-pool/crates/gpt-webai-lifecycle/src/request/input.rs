use std::path::PathBuf;
use std::time::Duration;

use crate::config::SupervisorConfig;
use crate::provider_runner::ProviderExecution;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};

use super::artifact_expectation::ArtifactExpectation;

#[derive(Clone, Debug)]
pub struct RequestRunInput {
    pub config: SupervisorConfig,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub pre_send_visual_gate: bool,
    pub pre_poll_wait_gate: bool,
    pub download_artifacts_after_poll: bool,
    pub artifact_expectation: ArtifactExpectation,
    pub prompt_file: PathBuf,
    pub files: Vec<PathBuf>,
    pub request_id: String,
    pub run_id: String,
    pub fencing_token: String,
    pub model: String,
    pub effort: String,
    pub ttl_ms: u128,
    pub send_retry_delays: Vec<Duration>,
    pub provider_limit_retry_delays: Vec<Duration>,
    pub send_process_timeout: Duration,
    pub poll_timeout_seconds: u64,
    pub poll_process_timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}
