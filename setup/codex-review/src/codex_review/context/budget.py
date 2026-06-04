"""Token estimation and bounded JSON serialization for model prompt assembly.

The pipeline feeds prompts to a model with a finite context window (~240k tokens),
so prompt builders need a shared, conservative token estimate plus a single
serializer that can bound its output. ``estimate_tokens``/``fit_to_budget`` provide
the estimate (tiktoken when available, else a chars/4 heuristic that intentionally
over-counts), and ``compact_json`` is the deterministic JSON serializer every
prompt builder should use instead of interpolating raw dicts.
"""
from __future__ import annotations

import json
from functools import lru_cache
from typing import Any

# Conservative bytes-per-token heuristic for English + source code.
CHARS_PER_TOKEN = 4

TRUNCATION_MARKER = "\n...[truncated]"


@lru_cache(maxsize=1)
def _encoding() -> "object | None":
    try:  # pragma: no cover - tiktoken is not a declared dependency in CI
        import tiktoken

        return tiktoken.get_encoding("cl100k_base")
    except Exception:
        return None


def estimate_tokens(text: str) -> int:
    """Estimate the token count of ``text`` (tiktoken if present, else chars/4)."""
    if not text:
        return 0
    enc = _encoding()
    if enc is not None:  # pragma: no cover - exercised only where tiktoken is installed
        try:
            return len(enc.encode(text))
        except Exception:
            pass
    return (len(text) + CHARS_PER_TOKEN - 1) // CHARS_PER_TOKEN


def tokens_to_chars(max_tokens: int) -> int:
    """Translate a token budget into an approximate character budget."""
    return max(0, int(max_tokens)) * CHARS_PER_TOKEN


def within_budget(text: str, max_tokens: int) -> bool:
    """True when ``text`` is estimated to fit inside ``max_tokens``."""
    return estimate_tokens(text) <= max_tokens


def fit_to_budget(text: str, max_tokens: int, *, marker: str = "\n...[truncated]") -> tuple[str, bool]:
    """Truncate ``text`` to an estimated ``max_tokens`` budget.

    Returns ``(text, truncated)``. The estimate is char-based, so the result is a
    conservative upper bound on tokens, never an under-count.
    """
    if max_tokens <= 0:
        return ("", bool(text))
    if within_budget(text, max_tokens):
        return (text, False)
    max_chars = max(0, tokens_to_chars(max_tokens) - len(marker))
    return (text[:max_chars] + marker, True)


def compact_json(value: Any, *, max_tokens: int | None = None, max_chars: int | None = None, indent: int | None = 2) -> str:
    """Serialize ``value`` to deterministic JSON, optionally truncated to a budget.

    ``max_chars`` takes precedence over ``max_tokens`` when both are given. When
    neither is set the full JSON is returned.
    """
    text = json.dumps(value, ensure_ascii=False, indent=indent, sort_keys=True)
    limit = max_chars if max_chars is not None else (tokens_to_chars(max_tokens) if max_tokens is not None else None)
    if limit is not None and len(text) > limit:
        return text[:limit] + TRUNCATION_MARKER
    return text
