use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gpt_webai_lifecycle::contracts::events::EventType;
use gpt_webai_lifecycle::contracts::provider::ProviderOperation;
use gpt_webai_lifecycle::provider_normalization_r12::events::{
    parse_r13_event_sequence, translate_retained_event, AdapterStage, RetainedEventResolution,
};
use gpt_webai_lifecycle::provider_normalization_r12::receipts::{
    parse_retained_receipts, validate_receipt_prefix, RetainedReceiptToken,
};
use gpt_webai_lifecycle::provider_normalization_r12::{
    load_crosswalk, RequiredProofOrReceipt, ResponsePolarity,
};

fn stack_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives below the stack root")
        .to_path_buf()
}

fn jsonl(relative: &str) -> Vec<Value> {
    fs::read_to_string(stack_root().join(relative))
        .expect("read immutable catalog")
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse immutable catalog record"))
        .collect()
}

fn checker(arguments: &[&str], fixture: Option<&Value>) -> Output {
    let root = stack_root();
    let mut command = Command::new("node");
    command
        .current_dir(&root)
        .arg(root.join("scripts/check-provider-normalization-r12.mjs"))
        .args(arguments);
    if let Some(record) = fixture {
        command
            .env(
                "GPT_WEBAI_FIXTURE_ID",
                record["fixture_id"].as_str().expect("fixture id"),
            )
            .env(
                "GPT_WEBAI_NORMALIZATION_SCHEMA",
                "pr72.provider_normalization.r12.v1",
            )
            .env("LANG", "C.UTF-8")
            .env("TZ", "UTC");
    }
    command.output().expect("execute R12 normalization checker")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn immutable_catalogs_pass_the_complete_aggregate_checker() {
    let output = checker(
        &[
            "--inventory",
            "contracts/provider-r12/provider-outcome-current.tsv",
            "--catalog",
            "contracts/provider-r12/provider-outcome-normalized.tsv",
            "--legal-catalog",
            "tests/fixtures/provider-r12/legal-catalog.jsonl",
            "--negative-catalog",
            "tests/fixtures/provider-r12/negative-catalog.jsonl",
            "--semantic-replay",
            "tests/fixtures/provider-r12/semantic-replay.jsonl",
        ],
        None,
    );
    assert!(
        output.status.success(),
        "checker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn generated_r13_crosswalk_is_deterministic_and_matches_every_retained_leaf() {
    let root = stack_root();
    let temp = temp_root("crosswalk-generation");
    let first = temp.join("first.tsv");
    let second = temp.join("second.tsv");
    for output in [&first, &second] {
        let generated = Command::new("node")
            .current_dir(&root)
            .arg(root.join("scripts/generate-r12-to-r13-crosswalk.mjs"))
            .arg("--output")
            .arg(output)
            .output()
            .expect("execute crosswalk generator");
        assert!(
            generated.status.success(),
            "generator failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        assert!(generated.stdout.is_empty());
        assert!(generated.stderr.is_empty());
    }

    let expected = fs::read(root.join("contracts/provider-r12/r12-to-r13-crosswalk.tsv"))
        .expect("read committed crosswalk");
    assert_eq!(fs::read(&first).unwrap(), expected);
    assert_eq!(fs::read(&second).unwrap(), expected);

    let catalog = load_crosswalk(
        &root.join("contracts/provider-r12/provider-outcome-normalized.tsv"),
        &first,
    )
    .expect("load generated crosswalk");
    assert_eq!(catalog.len(), 315);

    let uncertain = catalog.lookup("N074.base").unwrap();
    assert_eq!(uncertain.response.operation, ProviderOperation::SendClick);
    assert_eq!(uncertain.response.polarity, ResponsePolarity::Failure);
    assert_eq!(uncertain.required, RequiredProofOrReceipt::None);
    assert_eq!(uncertain.events, vec![EventType::SendUncertain]);
    assert_eq!(uncertain.lifecycle_result_kind, "run.send_uncertain");
    assert_eq!(uncertain.fail_closed_result_kind, "run.send_failed");

    for ordinal in (68..=72).chain(74..=87) {
        let row = catalog.lookup(&format!("N{ordinal:03}.base")).unwrap();
        assert_eq!(row.response.operation, ProviderOperation::SendClick);
        assert_eq!(row.response.polarity, ResponsePolarity::Failure);
        assert_eq!(row.required, RequiredProofOrReceipt::None);
    }

    for suffix in ["ae-none.artifact-zero", "ae-optional.artifact-zero"] {
        let zero = catalog.lookup(&format!("N026.{suffix}")).unwrap();
        assert_eq!(zero.response.operation, ProviderOperation::ArtifactDiscover);
        assert_eq!(zero.required, RequiredProofOrReceipt::ZeroControlProof);
        assert_eq!(zero.lifecycle_result_kind, "download.optional_zero");
    }

    for expectation in ["ae-claimed", "ae-none", "ae-optional", "ae-required"] {
        for artifact in ["artifact-empty", "artifact-nonempty"] {
            let visual = catalog
                .lookup(&format!("N030.{expectation}.{artifact}"))
                .unwrap();
            assert_eq!(visual.required, RequiredProofOrReceipt::None);
            assert_eq!(
                visual.lifecycle_result_kind, visual.fail_closed_result_kind,
                "visual_failure_reason must fail closed"
            );
        }
    }
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn legal_and_negative_commands_replay_exact_golden_bytes() {
    let legal = jsonl("tests/fixtures/provider-r12/legal-catalog.jsonl");
    let legal_record = &legal[0];
    let fixture_path = legal_record["fixture_path"].as_str().expect("fixture path");
    let output = checker(
        &[
            "--catalog",
            "contracts/provider-r12/provider-outcome-normalized.tsv",
            "--fixture",
            fixture_path,
        ],
        Some(legal_record),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout.len() as u64,
        legal_record["expected_stdout_byte_len"]
            .as_u64()
            .expect("legal stdout length")
    );
    assert_eq!(
        sha256(&output.stdout),
        legal_record["expected_stdout_sha256"]
            .as_str()
            .expect("legal stdout digest")
    );
    assert!(output.stderr.is_empty());

    let negative = jsonl("tests/fixtures/provider-r12/negative-catalog.jsonl");
    let negative_record = negative
        .iter()
        .find(|record| record["failure_class"] == "input.duplicate_operation")
        .expect("duplicate operation fixture");
    let input = negative_record["input_bytes_base64"]
        .as_str()
        .expect("negative input");
    let fixture_id = negative_record["fixture_id"]
        .as_str()
        .expect("negative fixture id");
    let output = checker(
        &["--negative-input-base64", input, "--fixture-id", fixture_id],
        Some(negative_record),
    );
    assert_eq!(output.status.code(), Some(70));
    assert_eq!(
        output.stdout.len() as u64,
        negative_record["expected_stdout_byte_len"]
            .as_u64()
            .expect("negative stdout length")
    );
    assert_eq!(
        sha256(&output.stdout),
        negative_record["expected_stdout_sha256"]
            .as_str()
            .expect("negative stdout digest")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn r23_crosswalk_loader_enforces_closed_serialization_and_leaf_parity() {
    let root = temp_root("crosswalk-loader");
    let normalized = root.join("normalized.tsv");
    let crosswalk = root.join("crosswalk.tsv");
    fs::write(
        &normalized,
        "normalized_leaf_id\tvalue\nN001.base\tone\nN002.base\ttwo\n",
    )
    .unwrap();
    fs::write(
        &crosswalk,
        concat!(
            "normalizedLeafId\tr13ResponseDiscriminant\trequiredProofOrReceipt\tr13EventSequence\tlifecycleResultKind\texit\tfailClosedResultKind\n",
            "N001.base\tcapture.root.failure\troot_binding_candidate\t-\tpreflight.schema_drift\t70\tpreflight.schema_drift\n",
            "N002.base\tstatus.success\tstatus_probe\tSlotHealthObserved\tstatus.ready\t0\tstatus.runtime_probe_failed\n",
        ),
    )
    .unwrap();

    let catalog = load_crosswalk(&normalized, &crosswalk).expect("valid closed crosswalk");
    assert_eq!(catalog.len(), 2);
    let first = catalog.lookup("N001.base").unwrap();
    assert_eq!(first.response.polarity, ResponsePolarity::Failure);
    assert_eq!(first.required, RequiredProofOrReceipt::RootBindingCandidate);
    assert!(first.events.is_empty());
    let second = catalog.lookup("N002.base").unwrap();
    assert_eq!(second.events, vec![EventType::SlotHealthObserved]);

    fs::write(
        &crosswalk,
        concat!(
            "normalizedLeafId\tr13ResponseDiscriminant\trequiredProofOrReceipt\tr13EventSequence\tlifecycleResultKind\texit\tfailClosedResultKind\n",
            "N002.base\tstatus.success\tstatus_probe\tSlotHealthObserved\tstatus.ready\t0\tstatus.runtime_probe_failed\n",
            "N001.base\tcapture.root.failure\troot_binding_candidate\t-\tpreflight.schema_drift\t70\tpreflight.schema_drift\n",
        ),
    )
    .unwrap();
    assert!(load_crosswalk(&normalized, &crosswalk).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retained_receipts_are_closed_and_prior_receipts_must_be_an_exact_prefix() {
    let tokens = parse_retained_receipts(
        "receipt.send.pre_click,receipt.send.post_click,receipt.send.start",
    )
    .unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0], RetainedReceiptToken::SendPreClick);
    validate_receipt_prefix(&tokens[..1], &tokens, 3).unwrap();
    assert!(validate_receipt_prefix(&tokens[1..2], &tokens, 3).is_err());
    assert!(parse_retained_receipts("receipt.unknown").is_err());
    assert!(parse_retained_receipts("receipt.status,receipt.status").is_err());
}

#[test]
fn retained_event_translation_preserves_stage_dependent_and_diagnostic_meaning() {
    assert_eq!(
        translate_retained_event("event.ProviderSessionUrlRejectedR12", AdapterStage::Rebind)
            .unwrap(),
        RetainedEventResolution::Event(EventType::SessionOperationFailed)
    );
    assert_eq!(
        translate_retained_event("event.ProviderSessionUrlRejectedR12", AdapterStage::Poll)
            .unwrap(),
        RetainedEventResolution::Event(EventType::PollFailed)
    );
    assert!(
        translate_retained_event("event.ProviderSessionUrlRejectedR12", AdapterStage::Other)
            .is_err()
    );
    assert_eq!(
        translate_retained_event("event.ProviderDownloadObservationR12", AdapterStage::Other)
            .unwrap(),
        RetainedEventResolution::Event(EventType::ArtifactRecoveryCandidateObserved)
    );
    assert_eq!(
        translate_retained_event("event.prior.ModelEnsureStarted", AdapterStage::Other).unwrap(),
        RetainedEventResolution::Event(EventType::ModelSelectionStarted)
    );
    assert_eq!(
        parse_r13_event_sequence("SendClicked,TurnStartConfirmed").unwrap(),
        vec![EventType::SendClicked, EventType::TurnStartConfirmed]
    );
    assert!(parse_r13_event_sequence("SendClicked,,TurnStartConfirmed").is_err());
}

fn temp_root(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gpt-webai-provider-normalization-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
