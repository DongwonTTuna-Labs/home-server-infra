"""Design cluster batch helpers."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json


def prioritize_clusters(clusters: dict[str, Any] | list[dict[str, Any]]) -> list[dict[str, Any]]:
    items=clusters.get("clusters", []) if isinstance(clusters, dict) else clusters
    return sorted(items, key=lambda c: (-(c.get("severity_score") or len(c.get("finding_ids", []))), str(c.get("cluster_id"))))


def make_cluster_batches(clusters: dict[str, Any], config: dict[str, Any]) -> list[dict[str, Any]]:
    items=prioritize_clusters(clusters)
    size=int(config.get("design", {}).get("max_cluster_analysis_batch_size", 4) or 4)
    return [{"schema_version":"design-cluster-batch.v1","batch_index":i//size,"clusters":items[i:i+size]} for i in range(0,len(items),size)] or [{"schema_version":"design-cluster-batch.v1","batch_index":0,"clusters":[]}]


def write_cluster_batch(batch: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, batch, "design-cluster-batch.v1")


def render_batch_summary(batches: list[dict[str, Any]]) -> str:
    return "\n".join(f"Batch {b.get('batch_index')}: {len(b.get('clusters', []))} clusters" for b in batches)
