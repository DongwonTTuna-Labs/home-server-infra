use std::fs;
use std::os::unix::fs::{symlink, DirBuilderExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::contracts::browser::{EvidenceMediaType, EvidenceRef};
use gpt_webai_lifecycle::contracts::ids::h256;
use gpt_webai_lifecycle::uploads::recovery::{
    classify_upload, validate_retry_after_clear, UploadOutcome, INCOMPLETE_REASON,
    STALE_MISMATCH_REASON, STALE_UNCLEARED_REASON,
};
use gpt_webai_lifecycle::uploads::staging::{
    stage_attachments, stage_prompt, AttachmentSource, StagingError,
};
use gpt_webai_lifecycle::uploads::{ChipProof, UploadProof};

#[test]
fn stages_request_scoped_records_with_closed_names_and_set_hash() {
    let fixture = Fixture::new();
    let source = fixture.write("Report.JSON", b"payload");
    let set = stage_attachments(
        fixture.path(),
        "request-1",
        "run-1",
        &[AttachmentSource {
            path: &source,
            media_type: "application/json",
        }],
    )
    .unwrap();

    assert_eq!(set.count, 1);
    assert!(set.records[0]
        .staged_rel_path
        .starts_with("requests/request-1/attachments/run-1/001-"));
    assert!(set.records[0].staged_rel_path.ends_with(".json"));
    assert!(set.records[0].container_rel_path.starts_with("run-1/001-"));
    assert_eq!(set.records[0].source_sha256, h256(b"payload"));
    set.validate_for("request-1", "run-1").unwrap();
}

#[test]
fn zero_and_sixty_four_files_are_valid_but_sixty_five_is_rejected() {
    let fixture = Fixture::new();
    let empty = stage_attachments(fixture.path(), "request-0", "run-0", &[]).unwrap();
    assert_eq!(empty.count, 0);
    empty.validate_for("request-0", "run-0").unwrap();

    let paths = (0..65)
        .map(|index| fixture.write(&format!("file-{index}.txt"), &[index as u8]))
        .collect::<Vec<_>>();
    let sixty_four = paths[..64]
        .iter()
        .map(|path| AttachmentSource {
            path,
            media_type: "text/plain",
        })
        .collect::<Vec<_>>();
    let set = stage_attachments(fixture.path(), "request-64", "run-64", &sixty_four).unwrap();
    assert_eq!(set.count, 64);

    let sixty_five = paths
        .iter()
        .map(|path| AttachmentSource {
            path,
            media_type: "text/plain",
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        stage_attachments(fixture.path(), "request-65", "run-65", &sixty_five),
        Err(StagingError::Invalid("attachment count"))
    ));
}

#[test]
fn unsafe_symlink_and_hardlink_sources_are_rejected() {
    let fixture = Fixture::new();
    let source = fixture.write("source.txt", b"bytes");
    let symbolic = fixture.path().join("symbolic.txt");
    symlink(&source, &symbolic).unwrap();
    assert!(matches!(
        stage_attachments(
            fixture.path(),
            "request-link",
            "run-link",
            &[AttachmentSource {
                path: &symbolic,
                media_type: "text/plain"
            }]
        ),
        Err(StagingError::UnsafeSource)
    ));

    let hard = fixture.path().join("hard.txt");
    fs::hard_link(&source, &hard).unwrap();
    assert!(matches!(
        stage_attachments(
            fixture.path(),
            "request-hard",
            "run-hard",
            &[AttachmentSource {
                path: &source,
                media_type: "text/plain"
            }]
        ),
        Err(StagingError::UnsafeSource)
    ));
}

#[test]
fn prompt_is_create_new_hash_bound_and_idempotent() {
    let fixture = Fixture::new();
    let prompt = b"exact prompt bytes\n";
    let digest = h256(prompt);
    let first = stage_prompt(fixture.path(), "request-p", "run-p", prompt, &digest).unwrap();
    let second = stage_prompt(fixture.path(), "request-p", "run-p", prompt, &digest).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.container_rel_path, "run-p/prompt.txt");
    assert!(matches!(
        stage_prompt(
            fixture.path(),
            "request-p2",
            "run-p2",
            prompt,
            &h256(b"different")
        ),
        Err(StagingError::Invalid("promptSha256"))
    ));
}

#[test]
fn classifies_complete_stale_retry_and_incomplete_proofs() {
    let fixture = Fixture::new();
    let source = fixture.write("one.bin", b"one");
    let set = stage_attachments(
        fixture.path(),
        "request-u",
        "run-u",
        &[AttachmentSource {
            path: &source,
            media_type: "application/octet-stream",
        }],
    )
    .unwrap();
    let current = ChipProof {
        chip_stable_key: h256(b"current-key"),
        label_hash: h256(b"current-label"),
        visible_size_bytes: Some(3),
        digest: Some(h256(b"one")),
        bounding_box_hash: h256(b"current-box"),
        complete: true,
        evidence_refs: vec![evidence("requests/r/operations/u/current.json")],
    };
    let mut proof = upload_proof(&set.set_sha256, 0, vec![current]);
    assert_eq!(
        classify_upload(&proof, &set, 1_020).unwrap(),
        UploadOutcome::Completed
    );

    proof.all_expected_complete = false;
    assert_eq!(
        classify_upload(&proof, &set, 1_020).unwrap(),
        UploadOutcome::Failed {
            reason: INCOMPLETE_REASON
        }
    );

    proof.all_expected_complete = true;
    proof.stale_chips.push(stale_chip());
    assert_eq!(
        classify_upload(&proof, &set, 1_020).unwrap(),
        UploadOutcome::MismatchObserved {
            reason: STALE_MISMATCH_REASON
        }
    );
    let mut retry = upload_proof(&set.set_sha256, 1, Vec::new());
    retry.upload_attempt_id = "upload-2".to_string();
    validate_retry_after_clear(&proof, &retry).unwrap();
    retry.stale_chips.push(stale_chip());
    assert_eq!(
        classify_upload(&retry, &set, 1_020).unwrap(),
        UploadOutcome::Failed {
            reason: STALE_UNCLEARED_REASON
        }
    );
}

#[test]
fn rejects_expired_or_set_mismatched_completion() {
    let fixture = Fixture::new();
    let set = stage_attachments(fixture.path(), "request-e", "run-e", &[]).unwrap();
    let proof = upload_proof(&set.set_sha256, 0, Vec::new());
    assert_eq!(
        classify_upload(&proof, &set, 31_001).unwrap(),
        UploadOutcome::Failed {
            reason: INCOMPLETE_REASON
        }
    );
    let mut mismatch = proof;
    mismatch.expected_set_sha256 = h256(b"other-set");
    assert!(classify_upload(&mismatch, &set, 1_001).is_err());
}

fn upload_proof(set_sha256: &str, retry_index: u8, chips: Vec<ChipProof>) -> UploadProof {
    UploadProof {
        upload_attempt_id: "upload-1".to_string(),
        retry_index,
        expected_set_sha256: set_sha256.to_string(),
        visible_current_chips: chips,
        stale_chips: Vec::new(),
        all_expected_complete: true,
        captured_at_ms: 1_000,
    }
}

fn stale_chip() -> ChipProof {
    ChipProof {
        chip_stable_key: h256(b"stale-key"),
        label_hash: h256(b"stale-label"),
        visible_size_bytes: None,
        digest: None,
        bounding_box_hash: h256(b"stale-box"),
        complete: false,
        evidence_refs: vec![evidence("requests/r/operations/u/stale.json")],
    }
}

fn evidence(path: &str) -> EvidenceRef {
    EvidenceRef {
        path: path.to_string(),
        sha256: h256(path.as_bytes()),
        size_bytes: 1,
        media_type: EvidenceMediaType::Json,
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "gpt-webai-lifecycle-upload-r13-{}-{nonce}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
