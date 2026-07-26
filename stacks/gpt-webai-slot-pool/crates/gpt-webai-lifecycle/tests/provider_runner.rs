use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::config::SupervisorConfig;
use gpt_webai_lifecycle::provider_runner::{
    DockerSlotProviderExecution, HostProviderExecution, ProviderCommandContext, ProviderExecution,
    ProviderPathMode, ProviderRunnerError, R13ProviderCommandContext,
};

#[test]
fn host_provider_execution_preserves_binary_prefix_and_env() {
    let execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: vec!["prefix".to_string()],
        env: vec![("A".to_string(), "B".to_string())],
    });

    let command = execution
        .command(ProviderCommandContext {
            config: &config(temp_state_root("host")),
            slot_id: "slot-01",
            run_id: "run-a",
        })
        .expect("host command");

    assert_eq!(command.provider_bin, PathBuf::from("/tmp/fake-provider"));
    assert_eq!(command.args_prefix, vec!["prefix"]);
    assert_eq!(command.env, vec![("A".to_string(), "B".to_string())]);
    assert_eq!(command.path_mode, ProviderPathMode::Host);
}

#[test]
fn docker_slot_provider_execution_builds_slot_pinned_docker_exec_command() {
    let root = temp_state_root("docker");
    let config = config(root.clone());
    let execution = ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: PathBuf::from("docker"),
        artifact_container_root: "/broker-artifacts".to_string(),
    });

    let command = execution
        .command(ProviderCommandContext {
            config: &config,
            slot_id: "slot-06",
            run_id: "run.with/slash",
        })
        .expect("docker command");

    assert_eq!(command.provider_bin, PathBuf::from("docker"));
    assert!(command
        .args_prefix
        .contains(&"gpt-webai-slot-06".to_string()));
    assert!(command
        .args_prefix
        .contains(&"gpt-webai-provider".to_string()));
    assert!(command
        .args_prefix
        .contains(&"BROWSER_AGENT_HOME=/state/slot-06".to_string()));
    assert!(command.args_prefix.contains(&"CDP_PORT=9228".to_string()));
    assert!(command
        .args_prefix
        .contains(&"GPT_WEBAI_ARTIFACTS_DIR=/broker-artifacts/run.with_slash".to_string()));
    assert!(command.args_prefix.iter().any(|arg| {
        arg.starts_with("GPT_WEBAI_ARTIFACTS_HOST_DIR=")
            && arg.contains("slot-06/artifacts/run.with_slash")
    }));
    assert!(root
        .join("slots/slot-06/artifacts/run.with_slash/downloads")
        .is_dir());
    assert!(root
        .join("slots/slot-06/attachments/run.with_slash")
        .is_dir());
    assert!(command.env.is_empty());
    let ProviderPathMode::DockerSlot(paths) = command.path_mode else {
        panic!("docker path mode");
    };
    assert_eq!(
        paths.artifact_host_dir,
        root.join("slots/slot-06/artifacts/run.with_slash")
    );
    assert_eq!(
        paths.artifact_container_dir,
        "/broker-artifacts/run.with_slash"
    );
    assert_eq!(
        paths.attachment_host_dir,
        root.join("slots/slot-06/attachments/run.with_slash")
    );
    assert_eq!(
        paths.attachment_container_dir,
        "/broker-attachments/run.with_slash"
    );
}

#[test]
fn docker_slot_provider_execution_rejects_unknown_slot() {
    let config = config(temp_state_root("missing"));
    let execution = ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: PathBuf::from("docker"),
        artifact_container_root: "/broker-artifacts".to_string(),
    });

    let error = execution
        .command(ProviderCommandContext {
            config: &config,
            slot_id: "slot-99",
            run_id: "run-a",
        })
        .expect_err("missing slot rejected");

    assert_eq!(
        error.to_string(),
        "slot not found for provider command: slot-99"
    );
}

#[test]
fn r13_host_command_uses_canonical_request_key_operation_root() {
    let root = temp_state_root("r13-host");
    let config = config(root.clone());
    let execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: vec!["provider.mjs".to_string()],
        env: vec![("LANG".to_string(), "C.UTF-8".to_string())],
    });

    let command = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "r-request-1",
            operation_id: "operation-1",
        })
        .expect("R13 host command");

    let operation_root = root.join("evidence/requests/r-request-1/operations/operation-1");
    assert_eq!(command.slot_id, "slot-01");
    assert_eq!(command.request_key, "r-request-1");
    assert_eq!(command.operation_id, "operation-1");
    assert_eq!(command.paths.operation_host_dir, operation_root);
    assert_eq!(
        command.paths.request_host_path,
        root.join("evidence/requests/r-request-1/operations/operation-1/provider-request.json")
    );
    assert_eq!(
        command.paths.request_container_path,
        command.paths.request_host_path
    );
    assert_eq!(
        command.paths.artifacts_host_dir,
        root.join("artifacts/r-request-1")
    );
    assert!(command.paths.operation_host_dir.is_dir());
    assert!(command.paths.artifacts_host_dir.is_dir());
}

#[test]
fn r13_diagnostic_key_is_bound_to_operation_id() {
    let config = config(temp_state_root("r13-diagnostic"));
    let execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: Vec::new(),
        env: Vec::new(),
    });

    let error = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-other-operation",
            operation_id: "operation-1",
        })
        .expect_err("mismatched diagnostic key");

    assert_eq!(
        error.to_string(),
        "invalid R13 provider command identity: diagnostic requestKey"
    );
}

#[test]
fn r13_host_command_rejects_non_allowlisted_environment_name() {
    let config = config(temp_state_root("r13-env"));
    let execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: Vec::new(),
        env: vec![("CDP_PORT".to_string(), "9223".to_string())],
    });

    let error = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-operation-1",
            operation_id: "operation-1",
        })
        .expect_err("legacy CDP env rejected");

    assert_eq!(
        error.to_string(),
        "provider environment name is not allowed: CDP_PORT"
    );
}

#[test]
fn r13_host_command_accepts_only_the_canonical_evidence_root_environment_names() {
    let config = config(temp_state_root("r13-evidence-env"));
    let execution = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: Vec::new(),
        env: vec![
            ("GPT_WEBAI_STATE_DIR".to_string(), "/state".to_string()),
            (
                "GPT_WEBAI_ARTIFACTS_DIR".to_string(),
                "/artifacts".to_string(),
            ),
            (
                "GPT_WEBAI_ARTIFACTS_HOST_DIR".to_string(),
                "/host-artifacts".to_string(),
            ),
        ],
    });

    let command = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-operation-1",
            operation_id: "operation-1",
        })
        .expect("canonical evidence environment accepted");
    assert_eq!(command.env, execution_env(&execution));

    let legacy_root = ProviderExecution::Host(HostProviderExecution {
        provider_bin: PathBuf::from("/tmp/fake-provider"),
        args_prefix: Vec::new(),
        env: vec![("GPT_WEBAI_STATE_ROOT".to_string(), "/state".to_string())],
    });
    let error = legacy_root
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-operation-1",
            operation_id: "operation-1",
        })
        .expect_err("lifecycle state-root variable is not a provider evidence variable");
    assert_eq!(
        error.to_string(),
        "provider environment name is not allowed: GPT_WEBAI_STATE_ROOT"
    );
}

fn execution_env(execution: &ProviderExecution) -> Vec<(String, String)> {
    match execution {
        ProviderExecution::Host(host) => host.env.clone(),
        ProviderExecution::DockerSlot(_) => unreachable!("host fixture"),
    }
}

#[test]
fn r13_host_command_rejects_symlinked_private_ancestors() {
    for component in ["evidence", "artifacts"] {
        let root = temp_state_root(&format!("r13-symlink-{component}"));
        fs::create_dir_all(&root).expect("state root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("state-root mode");
        let outside = temp_state_root(&format!("r13-outside-{component}"));
        fs::create_dir_all(&outside).expect("outside root");
        symlink(&outside, root.join(component)).expect("symlink private ancestor");
        let config = config(root);
        let execution = ProviderExecution::Host(HostProviderExecution {
            provider_bin: PathBuf::from("/tmp/fake-provider"),
            args_prefix: Vec::new(),
            env: Vec::new(),
        });

        let error = execution
            .r13_command(R13ProviderCommandContext {
                config: &config,
                slot_id: "slot-01",
                request_key: "d-operation-1",
                operation_id: "operation-1",
            })
            .expect_err("symlinked private ancestor rejected");

        assert!(matches!(error, ProviderRunnerError::Io(_)));
        assert_eq!(
            fs::read_dir(&outside).expect("outside entries").count(),
            0,
            "outside target must remain untouched"
        );
    }
}

#[test]
fn r13_docker_command_uses_canonical_slot_mount_mapping() {
    let root = temp_state_root("r13-docker-mapping");
    let config = config(root.clone());
    let execution = ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: PathBuf::from("docker"),
        artifact_container_root: "/broker-artifacts".to_string(),
    });

    let command = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-operation-1",
            operation_id: "operation-1",
        })
        .expect("R25 Docker mapping");

    assert_eq!(command.provider_bin, PathBuf::from("docker"));
    assert_eq!(
        command.paths.operation_host_dir,
        root.join("slots/slot-01/state/evidence/diagnostics/operation-1")
    );
    assert_eq!(
        command.paths.operation_container_dir,
        PathBuf::from("/state/slot-01/evidence/diagnostics/operation-1")
    );
    assert_eq!(
        command.paths.request_container_path,
        PathBuf::from("/state/slot-01/evidence/diagnostics/operation-1/provider-request.json")
    );
    assert_eq!(
        command.paths.artifacts_host_dir,
        root.join("artifacts/d-operation-1")
    );
    assert_eq!(
        command.paths.artifacts_container_dir,
        PathBuf::from("/broker-artifacts/d-operation-1")
    );
    assert!(command
        .args_prefix
        .windows(2)
        .any(|pair| { pair == ["--env", "GPT_WEBAI_STATE_DIR=/state/slot-01",] }));
    assert!(command.args_prefix.windows(3).any(|triple| {
        triple
            == [
                "gpt-webai-slot-01",
                "node",
                "provider/chatgpt-playwright/cli.mjs",
            ]
    }));
    assert!(command.env.is_empty());
}

#[test]
fn r13_docker_command_rejects_noncanonical_artifact_mount() {
    let config = config(temp_state_root("r13-docker-root"));
    let execution = ProviderExecution::DockerSlot(DockerSlotProviderExecution {
        docker_bin: PathBuf::from("docker"),
        artifact_container_root: "/downloads".to_string(),
    });

    let error = execution
        .r13_command(R13ProviderCommandContext {
            config: &config,
            slot_id: "slot-01",
            request_key: "d-operation-1",
            operation_id: "operation-1",
        })
        .expect_err("noncanonical artifact mapping rejected");
    assert_eq!(
        error.to_string(),
        "R13 Docker artifact container root must be /broker-artifacts"
    );
}

fn config(state_root: PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        state_root,
        slot_count: 10,
        slot_container_prefix: "gpt-webai-".to_string(),
        slot_mode: "docker".to_string(),
        status_provider_check: true,
        provider_status_timeout_ms: 1_000,
    }
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-provider-runner-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create state root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private state root");
    root
}
