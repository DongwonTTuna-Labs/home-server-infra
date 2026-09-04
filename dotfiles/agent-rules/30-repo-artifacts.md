## No Home-Grown Integrity Layers

Do not build, request, or restore self-made integrity mechanisms in a
git-tracked artifact repository: SHA-256 file-hash pins, byte-fixpoint
verification records (`VERIFY.json` and friends), file-count or file-list
contracts (`MANIFEST.json` and friends), tag-constant coupling, or self-checks
of git state.

The commit hash already content-addresses the whole tree, so the mechanism is
redundant; and an attacker who could defeat it could also edit the checker. The
defensive boundary for integrity and history is git history plus human and AI
review. The mechanism also costs a regenerate-and-reconverge cycle on every
edit.

Settled on 2026-08-23 by the user (RouteFork DA-10): rounds 24 through 26
produced ten blockers, every one of them a hole in the mechanism itself, and
review stopped converging. Do not raise its reintroduction as a blocker, even
in an adversarial review role.

Verification automation is limited to content checks that catch real mistakes:
reference integrity, count consistency, forbidden phrases, ledger row
integrity. A build-reproducibility check — regenerate a generated file and diff
it against the committed one — is a content check, not one of these layers.
