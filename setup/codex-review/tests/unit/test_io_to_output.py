import json

import pytest

from codex_review.cli import main


def test_io_to_output_emits_file_contents(tmp_path, monkeypatch, capsys):
    src = tmp_path / "lifecycle-result.json"
    payload = {"schema_version": "x", "lines": ["a", "b"], "nested": {"k": 'has "quotes"'}}
    # Pretty-print so the file genuinely spans multiple lines (heredoc path).
    src.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    out = tmp_path / "gh_output"
    monkeypatch.setenv("GITHUB_OUTPUT", str(out))

    rc = main(["io", "to-output", "--name", "lifecycle_result", "--in", str(src)])
    assert rc == 0

    written = out.read_text(encoding="utf-8")
    # Multiline content uses the heredoc form with a random delimiter.
    assert "lifecycle_result<<" in written
    # The exact JSON round-trips between the delimiter markers.
    body = written.split("\n", 1)[1].rsplit("\n", 2)[0]
    assert json.loads(body) == payload


def test_io_to_output_single_line_value(tmp_path, monkeypatch):
    src = tmp_path / "v.json"
    src.write_text('{"a":1}', encoding="utf-8")
    out = tmp_path / "gh_output"
    monkeypatch.setenv("GITHUB_OUTPUT", str(out))
    assert main(["io", "to-output", "--name", "v", "--in", str(src)]) == 0
    assert out.read_text(encoding="utf-8") == 'v={"a":1}\n'


def test_io_to_output_requires_name(tmp_path, capsys):
    src = tmp_path / "v.json"
    src.write_text("{}", encoding="utf-8")
    rc = main(["io", "to-output", "--in", str(src)])
    assert rc == 2
