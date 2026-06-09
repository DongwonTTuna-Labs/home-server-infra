from codex_review.memory.redaction import redact_memory_text
from codex_review.security.redaction import REDACTION_MARKER, assert_no_secret_patterns


def _fake_secret(prefix: str = "", size: int = 48) -> str:
    alphabet = "Aa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8Ii9Jj0KkLlMmNnPpQqRrSsTt9"
    repeats = (size // len(alphabet)) + 1
    return prefix + (alphabet * repeats)[:size]


def _assert_redacts_all(text: str, secrets: list[str]) -> str:
    redacted = redact_memory_text(text)
    for secret in secrets:
        assert secret not in redacted
    assert REDACTION_MARKER in redacted
    assert_no_secret_patterns(redacted, "memory")
    return redacted


def test_redact_memory_text_removes_github_token_prefixes():
    tokens = [_fake_secret(prefix) for prefix in ("ghp_", "gho_", "ghs_", "github_pat_")]
    text = "Memory mentions synthetic GitHub credentials: " + ", ".join(tokens)

    redacted = _assert_redacts_all(text, tokens)

    assert "Memory mentions synthetic GitHub credentials" in redacted


def test_redact_memory_text_removes_openai_and_relay_like_keys():
    keys = [_fake_secret("sk-"), _fake_secret("sk-proj-")]
    text = "Relay key rotation notes: " + " and ".join(keys)

    redacted = _assert_redacts_all(text, keys)

    assert "Relay key rotation notes" in redacted


def test_redact_memory_text_removes_aws_access_key_shape():
    aws_key = "AKIA" + "A1B2C3D4E5F6G7H8"
    text = f"AWS fixture for memory redaction: {aws_key}"

    redacted = _assert_redacts_all(text, [aws_key])

    assert "AWS fixture for memory redaction" in redacted


def test_redact_memory_text_removes_pem_private_key_blocks():
    pem_lines = [
        "-----BEGIN " + "RSA PRIVATE KEY-----",
        _fake_secret(size=64),
        _fake_secret(size=64),
        "-----END " + "RSA PRIVATE KEY-----",
    ]
    pem_block = "\n".join(pem_lines)
    text = f"Before key\n{pem_block}\nAfter key"

    redacted = _assert_redacts_all(text, [pem_block, *pem_lines])

    assert "Before key" in redacted
    assert "After key" in redacted


def test_redact_memory_text_removes_generic_high_entropy_strings():
    generic_secret = _fake_secret(size=72)
    text = f"Opaque integration credential observed in logs: {generic_secret}"

    redacted = _assert_redacts_all(text, [generic_secret])

    assert "Opaque integration credential observed in logs" in redacted


def test_redact_memory_text_removes_base64_like_high_entropy_strings():
    alphabet = "Aa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8Ii9Jj0KkLlMmNnPpQqRrSsTt9+/"
    base64_like_secret = (alphabet * 2)[:70] + "=="
    text = f"Opaque base64-like credential observed in logs: {base64_like_secret}"

    redacted = _assert_redacts_all(text, [base64_like_secret])

    assert "Opaque base64-like credential observed in logs" in redacted


def test_redact_memory_text_removes_multiline_secret_blocks():
    first_line = _fake_secret(size=32)
    second_line = _fake_secret(size=32)[::-1]
    text = f"Deployment note\nsecret:\n  {first_line}\n  {second_line}\nRetain this follow-up note"

    redacted = _assert_redacts_all(text, [first_line, second_line])

    assert "Deployment note" in redacted
    assert "Retain this follow-up note" in redacted


def test_redact_memory_text_preserves_benign_prose():
    text = (
        "Remember to cite existing files, keep the token budget useful, "
        "and explain why a finding is actionable before routing it."
    )

    assert redact_memory_text(text) == text


def test_redact_memory_text_preserves_pytest_absolute_temp_paths():
    path = (
        "/private/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/"
        "T/pytest-of-runner/pytest-135/"
        "test_axis_findings_normalize_a0/src/a.py"
    )
    text = f"Inspection evidence path: {path}"

    assert redact_memory_text(text) == text
    assert_no_secret_patterns(path, "memory.inspection_path")
