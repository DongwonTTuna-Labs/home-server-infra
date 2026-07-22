# GPT WebAI Slot Pool Live Smoke Tests

These smoke tests verify the slot pool at the provider boundary. They always
create real ChatGPT conversations through `gptpro`/`gpt-webai-lifecycle`.

The goal is to prove the things Docker health cannot prove:

- a selected slot is authenticated as ChatGPT Pro
- a new delegation creates a real `/c/...` conversation
- attached files reach the model context, not only the container or upload UI
- concurrent delegations allocate different slots and release them afterward
- session records and lifecycle status settle back to an operable state

Do not run this suite casually. It sends real prompts to ChatGPT and writes
evidence under `.omo/evidence/gpt-webai-slot-pool-live-smoke/<timestamp>/`.

## Commands

Run the default live QA suite:

```bash
stacks/gpt-webai-slot-pool/scripts/live-smoke.sh
```

The default suite is `qa-fast`: text-only, one attached file, three attached
files, resume, and width-5 parallel text/file/mixed checks.

Run a single case:

```bash
stacks/gpt-webai-slot-pool/scripts/live-smoke.sh --case live-attachment
```

Run parallel cases with a specific width:

```bash
stacks/gpt-webai-slot-pool/scripts/live-smoke.sh --case live-parallel-text --parallel 3
stacks/gpt-webai-slot-pool/scripts/live-smoke.sh --case live-parallel-attachment --parallel 2
```

Run the full width matrix:

```bash
stacks/gpt-webai-slot-pool/scripts/live-smoke.sh --case qa-full
```

## Live Cases

### Q1 `qa-fast`

Runs the default live QA suite:

- `live-text`
- `live-attachment`
- `live-attachments`
- `live-resume`
- `live-parallel-text --parallel 5`
- `live-parallel-attachment --parallel 5`
- `live-parallel-attachments --parallel 5`
- `live-parallel-mixed --parallel 5`

### Q2 `qa-full`

Runs the same base single-worker cases plus the width matrix for text,
single-file attachments, and multi-file attachments at `1`, `5`, and `10`
concurrent workers. It also runs a width-10 mixed text/multi-file case.

### L1 `live-text`

Proves a new text-only delegation reaches authenticated ChatGPT Pro.

Expected evidence:

- `gptpro` exits `0`
- JSON result has `status=complete`
- `answerText` contains the generated exact token
- result URL and session record contain a real `https://chatgpt.com/c/...`
  conversation URL
- session record has `kind=pro`, `model=pro`, `effort=extended`, `slotId`,
  and `status=done`
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

### L2 `live-attachment`

Proves one file attachment reaches the model context.

The script writes a unique `ATTACHMENT_CANARY.md`, attaches it with `gptpro
--file`, and requires the model to return the exact `CANARY_OK` line from the
file.

Expected evidence:

- result contains `CANARY_OK: <generated-token>`
- result does not contain `ATTACHMENT_MISSING`
- session record contains `attachmentCount=1`
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

### L3 `live-attachments`

Proves multiple file attachments reach the same model turn.

The script writes three unique canary files, attaches all of them with repeated
`gptpro --file` arguments, and requires the model to return every exact
`CANARY_OK_<n>` line.

Expected evidence:

- result contains all three generated canary tokens
- result does not contain `ATTACHMENT_MISSING`
- session record contains `attachmentCount=3`
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

### L4 `live-resume`

Proves an existing session can be resumed without creating a new send.

Expected evidence:

- the initial text delegation completes
- `gpt-webai-lifecycle resume --kind pro --session <sid>` returns the same
  answer token
- the original session record keeps the same `slotId` and remains `status=done`
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

### L5 `live-parallel-text`

Proves concurrent text delegations allocate independently and release state.

Expected evidence:

- every worker exits `0`
- every worker returns its own generated token
- session IDs are unique
- slot IDs are unique
- every session record is `status=done`
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

Default width is `5`. Use `--parallel 1`, `--parallel 5`, and `--parallel 10`
for the explicit width matrix.

### L6 `live-parallel-attachment`

Proves one-file attachment canaries still work under concurrent slot
allocation.

Expected evidence:

- every worker returns its own `CANARY_OK` token
- no worker returns `ATTACHMENT_MISSING`
- every session record contains `attachmentCount=1` and `status=done`
- slot IDs are unique
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

Default width is `5`.

### L7 `live-parallel-attachments`

Proves multi-file attachment canaries still work under concurrent slot
allocation.

Expected evidence:

- every worker returns all three of its own `CANARY_OK_<n>` tokens
- no worker returns `ATTACHMENT_MISSING`
- every session record contains `attachmentCount=3` and `status=done`
- slot IDs are unique
- final lifecycle status has all slots `ready`, `holders=0`, and `locks=0`

Default width is `5`.

### L8 `live-parallel-mixed`

Proves the broker can run text-only and multi-file attachment delegations in
the same concurrent wave.

Odd-numbered workers run text-only prompts. Even-numbered workers attach three
canary files. Expected evidence is the union of the text and multi-file
attachment checks, plus unique `slotId` values across the whole wave.

Default width is `5`.

## Evidence Rules

Evidence directories contain generated prompts, canary files, result JSON,
stderr, return codes, session IDs, slot IDs, and status snapshots.
Parallel cases write `final-status.out` even when a worker fails, so failures can
be separated into provider/send errors versus lifecycle state leaks.

They must not contain:

- Chrome profile files
- cookies or browser storage
- `$STATE/auth-seed/**`
- provider secrets or API keys

Keep evidence for failed runs. It is the primary way to distinguish container
file visibility, UI upload behavior, model-context attachment, and lifecycle
state release.
