## Anomalies

When something is wrong — a failing test, a type error, unexpected runtime
behavior, a suspicious log, a fix that only works by weakening a check —
diagnose before patching. Name the cause: implementation bug, test bug, spec
mismatch, environment, dependency, flaky timing, stale context, or a wrong
assumption. Then fix that cause, not the symptom.

If the problem is structural, fix the structure, and carry the fix through
docs, tests, config, the local machine, the home server, CI, and live smoke
checks when they are part of the same failure. Do not settle for a conservative
patch inside an approved scope.

A fallback is a safety net, never a reason to ship the weaker solution. When
something is uncertain, verify it or state the uncertainty — do not hide it.

## Tests

Tests are sensors. Never weaken, delete, skip, broaden, or fake one to reach a
green result. That includes changing expected values to match broken behavior,
blanket mocks that swallow failures, `@ts-ignore` or `as any` over a real type
error, empty catch blocks, and snapshot updates without evidence.

A test may change when the spec changed, the test is provably wrong, it is
flaky and being stabilized, it asserts implementation detail instead of
behavior, or its mock no longer matches the real contract. Say which one.

Decide test scope before writing code, and match effort to risk:

- No new tests for changes that carry no behavior and are trivially
  reversible: docs, comments, formatting, local renames, config already
  covered elsewhere.
- Add tests when behavior changes, when fixing a bug — write the failing case
  first — or when introducing a contract others depend on.
- Verify with the cheapest evidence that actually proves the behavior. Do not
  build harnesses, simulators, mock servers, or bespoke frameworks when a
  direct call or the existing suite proves the same thing. Building one
  requires an explicit instruction or a spec that mandates it.

## Reviewing Delegated Code

Reviewing code an implementer produced — a subagent, an external model, another
session — means judging architecture and idiom, not just whether it runs (user
instruction, 2026-09-04). Passing the gate is not the review; read the code
yourself and judge two things:

1. Module boundaries, duplication, and who owns which state.
2. Whether it is written the way the language, framework, and library are meant
   to be used. Rust: typed errors, serde extractors, compile-time checks such
   as sqlx `query_as!`. Svelte 5: runes, callback props, minimal effects.
   Postgres: constraints, indexes, range types.

Code that works while throwing away the tool's advantages is a defect. Send it
back through a plan addendum or a fix round rather than accepting it. When you
find something hand-rolled — a parser, a validator, a serializer — first check
whether the standard tooling already provides it.

## Implementation

Use the most focused complete diff that fixes the root cause and reaches the
target within the approved scope. Reuse existing helpers and patterns before
adding abstractions. Avoid speculative architecture, unrelated cleanup, silent
fallbacks, and hidden behavior changes.

Do not blend a retired plan into the current one. When a plan is replaced,
record the old one as retired and remove the dead path once it is safe.

If the requested architecture looks unsafe or wrong, say why before replacing
it — and if the user reaffirms it, build what they asked for.

## Pushing

Run `git push` only after the requested scope is complete, verification passed,
and no decision or blocker is outstanding. On this setup a push triggers
downstream automation, so a premature push is expensive, not merely untidy.

Immediately before pushing, confirm three things: the scope of the change, the
branch you are pushing to, and the verification result.

Never push an intermediate checkpoint, a partial fix, or unverified work. If
the user asks for a push while something is still unfinished or unverified, say
what is missing instead of pushing.
