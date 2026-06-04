from _pipeline import iter_all_steps


def test_every_upload_artifact_sets_short_retention():
    # Pipeline artifacts are intermediate handoffs, not long-term records. Leaving
    # retention-days unset means GitHub keeps them for 90 days, so every run of a
    # frequently-relabeled PR piles up. Require an explicit, short retention instead.
    uploads = [
        (job, step)
        for job, step in iter_all_steps()
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert uploads, "expected at least one upload-artifact step"
    for job, step in uploads:
        retention = (step.get("with") or {}).get("retention-days")
        assert retention is not None, f"{job}: upload-artifact must set retention-days"
        assert 1 <= int(retention) <= 14, f"{job}: retention-days should be short (<=14), got {retention}"
