use std::time::Duration;

mod flow;
mod output;

use crate::config::SupervisorConfig;
use crate::provider_runner::ProviderExecution;
use crate::request::artifact_expectation::ArtifactExpectation;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::runtime::{docker_runtime_for_provider, RuntimeProbe};

pub use output::DownloadOutput;

#[derive(Clone, Debug)]
pub struct DownloadInput {
    pub config: SupervisorConfig,
    pub session_id: String,
    pub fencing_token: String,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub artifact_expectation: Option<ArtifactExpectation>,
    pub provider_timeout: Duration,
    pub poll_timeout_seconds: u64,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

pub fn download_session(input: DownloadInput) -> DownloadOutput {
    let runtime = docker_runtime_for_provider(&input.config, &input.provider_execution);
    download_session_with_runtime(input, &runtime)
}

pub fn download_session_with_runtime(
    input: DownloadInput,
    runtime: &dyn RuntimeProbe,
) -> DownloadOutput {
    flow::run(input, runtime)
}
