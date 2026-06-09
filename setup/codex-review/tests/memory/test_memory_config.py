from __future__ import annotations

from pathlib import Path

import pytest

from codex_review.core.config import DEFAULT_CONFIG, load_config, validate_config
from codex_review.core.errors import ValidationError

SETUP_ROOT = Path(__file__).resolve().parents[2]
CONSUMER_OVERRIDE = SETUP_ROOT / "tests" / "fixtures" / "config" / "consumer_override.yml"


def test_trusted_core_exposes_safe_memory_defaults():
    config = load_config()

    assert config["context"]["memory_tokens"] == 6000
    assert config["memory"] == {
        "enabled": True,
        "root": ".omo/review-memory",
        "notepad_files": ["learnings.md", "decisions.md", "issues.md", "problems.md"],
        "ledger_file": "ledger.json",
        "max_entries": 200,
        "per_file_char_budget": 6000,
        "total_char_budget": 24000,
        "compaction_keep_recent_rounds": 3,
        "provenance_required_for_suppression": True,
        "hmac_env": "CODEX_MEMORY_HMAC_KEY",
    }
    assert config["memory"]["total_char_budget"] <= config["lifecycle"]["char_budget"]
    assert config["autofix"]["memory_write_prefix"] == ".omo/review-memory/"
    assert ".omo/" not in config["autofix"]["allowed_prefixes"]
    assert ".omo/review-memory/" not in config["autofix"]["allowed_prefixes"]


def test_consumer_override_deep_merges_memory_context_and_autofix_without_allowed_memory_prefix():
    config = load_config(
        override_path=CONSUMER_OVERRIDE,
        override={
            "context": {"memory_tokens": 4096},
            "memory": {"max_entries": 25, "total_char_budget": 12000},
            "autofix": {"memory_write_prefix": ".omo/review-memory/custom/"},
        },
    )

    assert config["context"]["memory_tokens"] == 4096
    assert config["context"]["total_patch_tokens"] == DEFAULT_CONFIG["context"]["total_patch_tokens"]
    assert config["memory"]["max_entries"] == 25
    assert config["memory"]["total_char_budget"] == 12000
    assert config["memory"]["root"] == ".omo/review-memory"
    assert config["autofix"]["memory_write_prefix"] == ".omo/review-memory/custom/"
    assert config["autofix"]["forbidden_files"] == ["Cargo.toml", "Cargo.lock"]
    assert ".omo/" not in config["autofix"]["allowed_prefixes"]
    assert ".omo/review-memory/" not in config["autofix"]["allowed_prefixes"]


def test_memory_section_is_required_mapping():
    config = load_config()
    config["memory"] = None

    with pytest.raises(ValidationError, match="config section 'memory' is required"):
        validate_config(config)


def test_memory_numeric_budgets_are_validated():
    config = load_config()
    config["memory"]["total_char_budget"] = config["lifecycle"]["char_budget"] + 1

    with pytest.raises(ValidationError, match="memory.total_char_budget must not exceed lifecycle.char_budget"):
        validate_config(config)


def test_context_memory_tokens_must_be_non_negative():
    config = load_config()
    config["context"]["memory_tokens"] = -1

    with pytest.raises(ValidationError, match="context.memory_tokens must be non-negative"):
        validate_config(config)
