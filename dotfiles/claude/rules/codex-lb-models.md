# Codex LB model selection

Which model to hand Codex when delegating — especially through orca
orchestration or a worktree handoff. The relay is the `model_providers.codex-lb`
block in `~/.codex/config.toml`.

Current default on both machines: **`gpt-6-astra`, effort `max`** (set by the
user on 2026-09-05; before that, sol at xhigh on the laptop and max on the home
server).

Endpoints differ by machine and the keys are not interchangeable: the laptop
goes to `https://relay-ai.dongwontuna.net/backend-api/codex` with
`CODEX_LB_LOCAL_API_KEY` (kept in `~/.codex/ai-relay.env`), the home server to
`http://127.0.0.1:2455/backend-api/codex` with `CODEX_LB_HOME_API_KEY` from the
login shell environment.

## The catalogue is per account, not per plan

The relay aggregates several ChatGPT accounts and fetches **each account's own
model catalogue** from upstream every five minutes. Two accounts on the same
Pro plan can be entitled to different models, so "the relay has model X" and
"the account that serves this request has model X" are different statements.

Measured 2026-09-05, four Pro accounts:

| model | accounts |
| --- | --- |
| `gpt-6-astra` | all 4 |
| `gpt-5.6-sol` / `terra` / `luna` | all 4 |
| `gpt-5.5`, `gpt-5.4-mini`, `gpt-5.3-codex-spark` | all 4 |
| `gpt-reserve`, `codex-auto-review` | all 4 |
| `gpt-5.4` | 3 of 4 — not `dongwon.lee.ai@gmail.com` |
| **`gpt-daybreak-blue-latest`** | **1 of 4 — only `ttuna0790@naver.com`** |

`GET /v1/models` returned ten ids on 2026-09-05: `gpt-6-astra`, `gpt-5.6-sol`,
`gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`,
`gpt-reserve`, `codex-auto-review`, `gpt-daybreak-blue-latest`. The account
catalogues additionally carry `gpt-5.3-codex-spark`, which the relay does not
advertise; `gpt-5.2` and `gpt-5.3-codex` are explicitly suppressed.

To re-check entitlement rather than guess — this is the authoritative source,
refreshed every five minutes:

```bash
ssh home 'docker exec codex-lb-postgres psql -U codex_lb -d codex_lb -Atc \
  "SELECT payload FROM model_registry_snapshot;"' \
  | python3 -c "import json,sys; s=json.load(sys.stdin)['snapshot']; \
print({k: len(v) for k, v in sorted(s['model_accounts'].items())})"
```

An unentitled model fails with HTTP 400 and this exact message, produced
upstream and only pattern-matched by codex-lb:

```
The '<model>' model is not supported when using Codex with a ChatGPT account.
```

`gpt-6-astra-aeon` and `gpt-5.4-nano` both answer that way today. The rejection
names the model, not the account, and codex-lb keeps it out of account health —
but it does not fail over to a different account, so a one-account model has no
redundancy.

## Choosing a model

| model | priority | Fast tier | reasoning | use for |
| --- | --- | --- | --- | --- |
| `gpt-6-astra` | 1 | 2x | low–ultra, default medium | the default; hard work |
| `gpt-reserve` | 3 | 1.5x | low–max | cheap and fast |
| `gpt-5.6-sol` | 6 | 1.5x | low–ultra, default low | the previous default |
| `gpt-daybreak-blue-latest` | 10 | none | low–ultra, default low | defensive security |

All four carry a 272k context window with an 872k `max_context_window`.
Reasoning levels come from the model metadata and are identical on both
machines — the endpoint does not change what efforts exist.

`gpt-6-astra` requires **Codex CLI 0.153.0 or newer** (`minimal_client_version`).
An older client still gets an answer but logs `Model metadata for 'gpt-6-astra'
not found` and falls back to generic metadata, which degrades it. Both machines
run 0.153.3.

### sol

Call sol at effort `max` (user instruction, 2026-08-23: "다음부터는 sol max
로"). Always pass it explicitly: `-c model_reasoning_effort='"max"'`.

### Daybreak Blue — defensive security work

Vendor-designated for incident response, malware triage, detection rules,
security audit, and vulnerability analysis. Use it when handing that kind of
work to Codex — but read the evidence before believing it is smarter.

Measured 2026-08-22 against sol; **no capability difference was found**:

- Auditing vulnerable Flask code: both found the same seven issues (SQLi,
  command injection, path traversal, debug exposure, weak hashing, hardcoded
  credentials, access control). Only severity labels differed slightly.
- IR triage of a compromised shell script: neither refused, ATT&CK mappings
  matched, and both caught the subtle point that `$P` is not exported through a
  single-quoted `sh -c`, so the C2 loop fails.
- No difference in refusals or over-caution. sol does defensive security work
  without hedging.
- The one consistent difference was speed: at equal effort daybreak was 30-40%
  faster (audit 24.4s vs 30.3s, IR 41.5s vs 61.9s) at near-identical token use.
- sol answered in Korean and included concrete cleanup commands, which was more
  useful.

So do not say daybreak is better at security. It is the vendor's designated
model and was faster at equal quality; that is the whole of the evidence. When
quality is what matters, sol is an equal choice.

Two current constraints: daybreak has **no Fast tier at all** (empty
`additional_speed_tiers` and `service_tiers`) and the lowest priority of the
four, and it is entitled on **only one account**, so it has no failover if that
account hits a limit. Do not use it for ordinary coding, refactoring, or design.

## Invoking

`orca worktree create --agent codex` does not accept Codex's `--model` or
`-c model_reasoning_effort`. Create the worktree, then put the command into the
terminal:

```text
orca worktree create --name <task> --no-parent --json
orca terminal create --worktree id:<repoId>::<path> --title <task> \
  --command 'codex --model gpt-daybreak-blue-latest -c model_reasoning_effort="high"' --json
orca terminal wait --terminal <handle> --for tui-idle --timeout-ms 60000 --json
orca terminal send --terminal <handle> --text "<task brief>" --enter --json
```

One-shot, non-interactive:

```bash
codex exec --model gpt-6-astra -c model_reasoning_effort='"max"' "<prompt>"
```

- Backgrounding `codex exec` with `&` **requires `< /dev/null`**; without it the
  process hangs at `Reading additional input from stdin...`.
- Outside a trusted directory, `codex exec` refuses until you pass
  `--skip-git-repo-check`.
- `-c` values are parsed as TOML, so a string needs doubled quoting:
  `-c model_reasoning_effort='"max"'`.

## Korean output from sol-family models

Asking sol for Korean documents, reports, or comments reliably produces English
words and English sentence shapes that Korean speakers do not actually use,
which makes the result hard to read (user complaint, 2026-08-23).

- Include this in the prompt whenever the deliverable is Korean: "한국인이 실제
  업무에서 쓰는 자연스러운 한국어로만 작성하라. 정착된 외래어는 허용하되,
  현지인이 잘 쓰지 않는 영단어·영어식 표현은 전부 한국어로 바꿔라."
- Apply the same bar when reviewing what comes back: correct the unnaturalized
  English, or send the correction back on the same thread.
- Do not over-purify. Loanwords that are established in Korean — 서버, 버튼,
  리뷰, 커밋 — stay.

## Invariants

- Never guess a model name. If it is not in the catalogue the call is a 400.
- Never print, commit, or transmit the relay API keys
  (`CODEX_LB_LOCAL_API_KEY`, `CODEX_LB_HOME_API_KEY`, `~/.codex/ai-relay.env`).
- Security work does not automatically force daybreak. Choose within the
  evidence above, and if the user names a model, use it.
- This file records measurements with dates. When a number matters, re-measure
  instead of trusting the table.
