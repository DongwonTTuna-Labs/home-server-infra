"""Tests for the design analysis matrix fan-out (Workstream B)."""
from __future__ import annotations

import json
from pathlib import Path

from codex_review.cli import main

CONFIG = str(Path(__file__).resolve().parents[2] / "config.yml")


def _clusters(n):
    return {
        "schema_version": "design-clusters.v1",
        "clusters": [{"cluster_id": f"c{i}", "finding_ids": [f"f{i}"], "summary": f"cluster {i}"} for i in range(n)],
        "cluster_count": n,
    }


def test_prepare_analysis_matrix_batches_clusters(tmp_path):
    clusters_file = tmp_path / "clusters.json"
    clusters_file.write_text(json.dumps(_clusters(6)), encoding="utf-8")
    ctx_file = tmp_path / "ctx.json"
    ctx_file.write_text(json.dumps({"schema_version": "design-context.v1"}), encoding="utf-8")
    out = tmp_path / "matrix.json"
    rc = main([
        "design", "prepare-analysis-matrix", "--config", CONFIG,
        "--in", str(clusters_file), "--pr-context", str(ctx_file),
        "--work-dir", str(tmp_path / "batches"), "--repo-path", "pr-head", "--out", str(out),
    ])
    assert rc == 0
    matrix = json.loads(out.read_text(encoding="utf-8"))
    # 6 clusters with batch size 4 -> 2 parallel analysis jobs.
    assert len(matrix["include"]) == 2
    indexes = sorted(e["batch_index"] for e in matrix["include"])
    assert indexes == [0, 1]
    for entry in matrix["include"]:
        assert Path(entry["prompt_file"]).exists()
        assert Path(entry["batch_file"]).exists()
        assert entry["working_directory"] == "pr-head"
        assert entry["validated_file"].endswith("analysis.validated.json")
        batch = json.loads(Path(entry["batch_file"]).read_text(encoding="utf-8"))
        assert batch["clusters"], "each batch carries at least one cluster"


def test_prepare_analysis_matrix_empty_when_no_clusters(tmp_path):
    clusters_file = tmp_path / "clusters.json"
    clusters_file.write_text(json.dumps(_clusters(0)), encoding="utf-8")
    out = tmp_path / "matrix.json"
    rc = main([
        "design", "prepare-analysis-matrix", "--config", CONFIG,
        "--in", str(clusters_file), "--work-dir", str(tmp_path / "batches"), "--out", str(out),
    ])
    assert rc == 0
    assert json.loads(out.read_text(encoding="utf-8"))["include"] == []


def test_collect_analyses_merges_per_batch_validated_files(tmp_path):
    root = tmp_path / "downloads"
    for idx, cid in enumerate(["c0", "c1"]):
        d = root / f"codex-review-design-analysis-{idx}"
        d.mkdir(parents=True)
        (d / "analysis.validated.json").write_text(
            json.dumps({"schema_version": "design-cluster-analysis.v1", "analyses": [{"cluster_id": cid, "status": "fix_now"}]}),
            encoding="utf-8",
        )
        # raw output must NOT be collected
        (d / "analysis.raw.json").write_text(json.dumps({"analyses": [{"cluster_id": "RAW", "status": "x"}]}), encoding="utf-8")
    out = tmp_path / "collected.json"
    rc = main(["design", "collect-analyses", "--config", CONFIG, "--artifacts", str(root), "--out", str(out)])
    assert rc == 0
    collected = json.loads(out.read_text(encoding="utf-8"))
    assert collected["analysis_count"] == 2
    assert {a["cluster_id"] for a in collected["analyses"]} == {"c0", "c1"}
