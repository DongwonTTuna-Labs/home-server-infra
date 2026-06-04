"""First-class issue fallback for work the PR loop cannot safely mutate."""
from __future__ import annotations

import hashlib
from typing import Any

from codex_review.core.errors import ValidationError
from codex_review.github.issues import create_or_update_deferred_issue
from codex_review.github.markers import render_marker

SCHEMA_VERSION = "issue-fallback-issue-fallback.v1"
CONTENT_SCHEMA_VERSION = "issue-fallback-content.v1"

# Machine reason keys stay English/snake_case; user-facing labels are Korean.
_REASON_LABELS_KO: dict[str, str] = {
    "missing_openspec_spec": "OpenSpec 변경 명세 누락",
    "unresolved_openspec_source": "OpenSpec 소스 미해결",
    "fork_pr_push_blocked": "포크 PR 푸시 차단",
    "out_of_scope": "PR 범위 밖 작업",
    "techlead_defer_to_issue": "리뷰 지적사항 이슈 이관",
    "no_diff_repeat": "빈 패치 반복",
    "no-diff-repeat": "빈 패치 반복",
    "empty_patch": "빈 패치",
    "oscillation_detected": "자동 수정 진동 감지",
    "max_rounds_reached": "최대 반복 횟수 초과",
    "review_needs_human": "리뷰 단계 사람 개입 필요",
    "design_needs_human": "설계 단계 사람 개입 필요",
    "stop_needs_human": "사람 개입 필요",
    "artifacts_missing": "이전 단계 산출물 누락",
    "manual_fallback": "수동 후속조치 필요",
}


def reason_label_ko(reason: str) -> str:
    return _REASON_LABELS_KO.get(reason, "후속조치 필요")


def _repository(pr_context: dict[str, Any]) -> str:
    repository = pr_context.get("repository") or pr_context.get("base_repo_full_name")
    if repository:
        return str(repository)
    return f"{pr_context.get('owner')}/{pr_context.get('repo')}"


def _pr_reference(pr_context: dict[str, Any]) -> str:
    if pr_context.get("html_url"):
        return str(pr_context["html_url"])
    number = pr_context.get("pr_number")
    return f"#{number}" if number else "unknown PR"


def _key_material(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any], deferred_items: list[dict[str, Any]] | None = None) -> str:
    sources = ",".join(str(x) for x in openspec_context.get("source_summary", []))
    deferred = ",".join(
        str(item.get("root_cause_key") or item.get("finding_id") or item.get("id") or item.get("title") or "deferred")
        for item in (deferred_items or [])
    )
    return f"{_repository(pr_context)}#{pr_context.get('pr_number')}:{reason}:{sources}:{deferred}"


def make_issue_fallback_key(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any], deferred_items: list[dict[str, Any]] | None = None) -> str:
    return hashlib.sha256(_key_material(reason, pr_context, openspec_context, deferred_items).encode("utf-8")).hexdigest()[:24]


def render_issue_fallback_body(
    *,
    idempotency_key: str,
    reason: str,
    pr_context: dict[str, Any],
    openspec_context: dict[str, Any],
    attempted_stages: list[str],
    required_follow_up: str,
    deferred_items: list[dict[str, Any]] | None = None,
) -> str:
    lines = [
        render_marker("codex-review:issue-fallback", {"key": idempotency_key, "reason": reason}),
        "# Codex 리뷰 후속조치",
        "",
        f"사유: `{reason}` ({reason_label_ko(reason)})",
        f"원본 PR: {_pr_reference(pr_context)}",
        "",
        "## OpenSpec 소스",
    ]
    sources = openspec_context.get("source_summary") or []
    if sources:
        lines.extend(f"- {source}" for source in sources)
    else:
        lines.append(f"- `{openspec_context.get('decision') or 'missing_openspec_spec'}`")
    deferred = deferred_items or []
    if deferred:
        lines.extend(["", "## 이관된 지적사항"])
        for item in deferred:
            finding = item.get("finding_id") or item.get("id") or "unknown"
            title = item.get("title") or item.get("summary") or "이관된 리뷰 항목"
            location = item.get("file") or item.get("path") or ""
            line = item.get("line")
            where = f" ({location}:{line})" if location and line else (f" ({location})" if location else "")
            root = item.get("root_cause_key") or item.get("root_cause") or ""
            root_text = f"; 근본 원인 `{root}`" if root else ""
            recommendation = item.get("recommendation") or item.get("reason") or item.get("summary") or "현재 PR 수정 루프 밖에서 처리하세요."
            lines.append(f"- `{finding}`{where}: {title}{root_text}. 후속: {recommendation}")
    lines.extend(["", "## 시도한 단계"])
    lines.extend(f"- {stage}" for stage in attempted_stages or ["unknown"])
    lines.extend(["", "## 필요한 후속조치", required_follow_up])
    return "\n".join(lines)


def build_issue_fallback_plan(
    *,
    reason: str,
    pr_context: dict[str, Any],
    openspec_context: dict[str, Any],
    attempted_stages: list[str] | None = None,
    required_follow_up: str | None = None,
    deferred_items: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    key = make_issue_fallback_key(reason, pr_context, openspec_context, deferred_items)
    follow_up = required_follow_up or _default_follow_up(reason, openspec_context)
    body = render_issue_fallback_body(
        idempotency_key=key,
        reason=reason,
        pr_context=pr_context,
        openspec_context=openspec_context,
        attempted_stages=attempted_stages or [],
        required_follow_up=follow_up,
        deferred_items=deferred_items or [],
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "planned",
        "reason": reason,
        "idempotency_key": key,
        # Korean, human-facing title; reason key kept for traceability/search.
        "title": f"[Codex 리뷰 후속조치] {reason_label_ko(reason)} ({reason})",
        "body": body,
        "required_follow_up": follow_up,
        "attempted_stages": attempted_stages or [],
        "openspec_sources": openspec_context.get("source_summary") or [],
        "deferred_items": deferred_items or [],
        "deferred_count": len(deferred_items or []),
    }


def _default_follow_up(reason: str, openspec_context: dict[str, Any]) -> str:
    if reason in {"missing_openspec_spec", "unresolved_openspec_source"} or not openspec_context.get("present"):
        return "PR 제목/본문에 OpenSpec 변경 산출물을 추가하거나 링크한 뒤 Codex 리뷰를 다시 실행하세요."
    if reason in {"fork_pr_push_blocked", "out_of_scope"}:
        return "구현을 동일 저장소 브랜치로 옮기거나, 범위 밖 작업은 별도 PR에서 처리하세요."
    if reason == "techlead_defer_to_issue":
        return "현재 PR 수정 범위 밖이지만 유효한 작업에 대해 후속 이슈를 생성/갱신하고, 구현 가능한 항목은 PR 수정 루프를 계속 진행하세요."
    if reason in {"no-diff-repeat", "no_diff_repeat", "empty_patch"}:
        return "생성된 수정 산출물을 점검하고, 다음 실행에서 비어있지 않은 패치가 나오도록 OpenSpec 작업 또는 구현 계획을 조정하세요."
    if reason in {"oscillation_detected", "max_rounds_reached"}:
        return "자동 수정 루프가 같은 문제를 반복 수정(또는 반복 횟수 상한 도달)하며 수렴하지 못해 중단되었습니다. 사람이 상충하는 리뷰 피드백을 조율해 직접 수정한 뒤 Codex 리뷰를 다시 실행하세요."
    if reason in {"design_needs_human", "review_needs_human", "stop_needs_human"}:
        return "자동화가 안전하게 진행할 수 없는 비실행 블로커가 있습니다. 사람이 판단해 처리한 뒤 Codex 리뷰를 다시 실행하세요."
    if reason == "artifacts_missing":
        return "이전 단계 산출물(artifact)이 만료/누락되었습니다. 처음부터 다시 `리뷰중` 라벨을 부착해 파이프라인을 재시작하세요."
    return "차단 조건을 해소한 뒤 Codex 리뷰를 다시 실행하세요."


def infer_issue_reason(
    *,
    review_publication: dict[str, Any] | None = None,
    design_route: dict[str, Any] | None = None,
    fix_validation: dict[str, Any] | None = None,
    fallback_reason: str | None = None,
) -> dict[str, Any]:
    """Derive an issue reason key from the artifacts of prior workflow stages.

    Priority: terminal fix-loop conditions first, then design/review needs-human,
    then deferred review items, finally an explicit/manual fallback.
    """
    attempted: list[str] = []
    deferred_items: list[dict[str, Any]] = []

    fix = fix_validation or {}
    if fix:
        attempted.append("push")
    terminal = str(fix.get("loop_terminal_reason") or "").strip()
    status = str(fix.get("status") or "").strip()
    for candidate in (terminal, status):
        if candidate in {"oscillation_detected", "max_rounds_reached", "no_diff_repeat", "no-diff-repeat", "empty_patch"}:
            return {"reason": candidate, "attempted_stages": attempted, "deferred_items": deferred_items}

    design = design_route or {}
    if design:
        attempted.append("design_chief")
        route = str(design.get("route") or design.get("status") or "").strip()
        if route in {"stop_needs_human", "needs_human", "design_needs_human"}:
            return {"reason": "design_needs_human", "attempted_stages": attempted, "deferred_items": deferred_items}

    review = review_publication or {}
    if review:
        attempted.append("techlead")
        items = review.get("deferred_items")
        if isinstance(items, list) and items:
            deferred_items = items
            return {"reason": "techlead_defer_to_issue", "attempted_stages": ["techlead_defer_to_issue"], "deferred_items": deferred_items}
        status02 = str(review.get("status") or "").strip()
        if status02 in {"needs_human", "review_needs_human"}:
            return {"reason": "review_needs_human", "attempted_stages": attempted, "deferred_items": deferred_items}

    reason = (fallback_reason or "").strip() or ("artifacts_missing" if not (fix or design or review) else "manual_fallback")
    return {"reason": reason, "attempted_stages": attempted, "deferred_items": deferred_items}


def build_issue_content_prompt(plan: dict[str, Any]) -> str:
    """Prompt asking the model to compose a concise Korean title and a polished Korean body.

    The deterministic plan body is the source of truth for facts; the model only
    rewrites it into clearer Korean prose. Machine fields (reason, marker) must be preserved.
    """
    return "\n".join(
        [
            "역할: 너는 Codex 리뷰 파이프라인의 후속조치 이슈를 작성하는 보조자다.",
            "아래 결정적으로 생성된 이슈 초안을 바탕으로, 사람이 읽기 좋은 한국어 이슈를 정리하라.",
            "",
            "규칙:",
            "- 제목(title)은 한국어 한 줄로 핵심을 요약한다. 50자 이내 권장.",
            "- 본문(body)은 한국어로 작성한다. 초안의 사실(사유 키, PR 참조, OpenSpec 소스, 이관 항목, 후속조치)을 빠짐없이 포함한다.",
            "- 본문 첫 줄의 HTML 주석 마커(<!-- ... -->)는 그대로 유지한다. 멱등성 키이므로 절대 수정/삭제하지 않는다.",
            f"- 기계용 사유 키 `{plan.get('reason')}` 는 본문 어딘가에 그대로 남긴다.",
            "- 추측하지 말고 초안에 있는 정보만 사용한다.",
            "",
            f'출력은 JSON 한 개: {{"schema_version": "{CONTENT_SCHEMA_VERSION}", "title": "...", "body": "..."}}',
            "",
            "## 이슈 초안 (제목)",
            str(plan.get("title") or ""),
            "",
            "## 이슈 초안 (본문)",
            str(plan.get("body") or ""),
        ]
    )


def compose_issue_content(plan: dict[str, Any], model_content: dict[str, Any] | None) -> dict[str, Any]:
    """Merge model-polished title/body over the deterministic plan, with safe fallback.

    The model output is only trusted when it is non-empty and preserves the
    idempotency marker; otherwise the deterministic Korean plan is kept.
    """
    plan = dict(plan)
    content = model_content or {}
    title = str(content.get("title") or "").strip()
    body = str(content.get("body") or "").strip()
    marker = render_marker("codex-review:issue-fallback", {"key": plan.get("idempotency_key"), "reason": plan.get("reason")})
    marker_token = marker.split(" ", 2)[1] if " " in marker else "codex-review:issue-fallback"
    if title:
        plan["title"] = title
    if body and (marker_token in body or "codex-review:issue-fallback" in body):
        plan["body"] = body
    else:
        # Model dropped the marker (or returned nothing): keep the deterministic body so
        # idempotency is never broken.
        if not body:
            plan["body_polish_skipped"] = "empty_model_body"
        else:
            plan["body_polish_skipped"] = "missing_marker"
    return plan


def apply_issue_fallback(plan: dict[str, Any], pr_context: dict[str, Any], token: str | None, *, dry_run: bool) -> dict[str, Any]:
    owner = pr_context.get("owner")
    repo = pr_context.get("repo")
    if dry_run:
        return {**plan, "status": "dry_run", "issue_url": None}
    if not token:
        raise ValidationError("issue_fallback actual issue fallback requires a GitHub App installation token")
    result = create_or_update_deferred_issue(str(owner), str(repo), plan["idempotency_key"], plan["title"], plan["body"], token)
    return {**plan, "status": "applied", "issue_url": result.get("html_url") or result.get("url")}
