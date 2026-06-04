from codex_review.loop.router import route_after_resolve_gate, route_after_techlead, route_after_design_chief, route_after_push


def test_routes_across_key_stages():
    assert route_after_resolve_gate({"route":"run_review"})["run_review"] is True
    assert route_after_techlead({"status":"needs_design", "decisions":[{"action":"needs_design", "normalized_from":"needs_human"}]})["route"] == "run_design"
    assert route_after_techlead({"decisions":[{"action":"needs_human", "blocker_type":"secret_required"}]})["route"] == "stop_needs_human"
    assert route_after_design_chief({"status":"approved_for_fix"})["route"] == "run_fix_dispatch"
    assert route_after_push({"pushed":True})["route"] == "record_reentry"
