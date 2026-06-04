# Workflow Contract Tests

Run the lightweight GitHub Actions workflow contract harness with:

```bash
uvx pytest tests/workflows -q
```

The harness skips the expected codex-loop workflow files until they are added, then validates trigger presence, the dispatch workflow's `codex-loop` repository dispatch event type marker, minimal explicit permissions, payload schema guard markers, and absence of label/comment orchestration commands. Negative fixtures under `tests/workflows/fixtures/` prove that `permissions: write-all` and label mutation commands are rejected without creating real workflow files.
