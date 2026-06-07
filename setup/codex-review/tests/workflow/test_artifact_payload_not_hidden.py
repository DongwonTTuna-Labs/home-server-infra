"""Published artifact payloads must never be hidden (dotfile) paths.

`actions/upload-artifact@v4` excludes hidden files by default (since v4.4), so
uploading a path whose basename starts with `.` silently produces an EMPTY
artifact and any downstream download then 404s. Loop state is handed off this
way between jobs, so a hidden artifact payload breaks the loop. Same-job
scratch dotfiles that are never uploaded are fine; this guard only constrains
what crosses the artifact boundary.
"""
from pathlib import Path

from _pipeline import iter_all_steps


def _upload_paths():
    for job, step in iter_all_steps():
        if str(step.get("uses", "")).startswith("actions/upload-artifact@"):
            raw = (step.get("with") or {}).get("path")
            if raw is None:
                continue
            for line in str(raw).splitlines():
                entry = line.strip().lstrip("!")
                if entry:
                    yield job, entry


def test_no_uploaded_artifact_path_is_a_hidden_file():
    offenders = [
        (job, entry)
        for job, entry in _upload_paths()
        if Path(entry).name.startswith(".")
    ]
    assert not offenders, (
        "upload-artifact paths must not be hidden files (excluded by default, "
        f"yielding empty artifacts): {offenders}"
    )
