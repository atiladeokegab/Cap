import math

import pytest
from conftest import file_snapshot, listed, validate_contract

ROW = {"shot_id", "one_liner", "performer_label", "budget_s", "budget_label"}


def full():
    # 0.6.0 fields; validate once vendored.
    return {
        "shots": [
            {
                "shot_id": sid,
                "title": title,
                "lines": [{"sentence_id": f"{sid}-1", "text": f"{title} now."}],
                "budget_s": budget,
                "actions": actions,
                "performer_label": performer,
            }
            for sid, title, budget, actions, performer in (
                ("intro", "Say what it is", 20, [], "Speaker 1"),
                ("demo", "Show the export", 30, ["Click Export"], "Speaker 2"),
                ("ask", "Ask for the vote", 25, [], "Presenter"),
            )
        ]
    }


def overview(driver, tmp_path, full_list, current, limit):
    """The overview, opened from the teleprompter before recording; the
    teleprompter shows the pick-up list (current.json), the overview full.json."""
    before = file_snapshot(tmp_path)
    result = driver(
        "prompter",
        shotlist=current,
        full_shotlist=full_list,
        event={"limit": limit},
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 100, "action": "overview"}],
    )
    assert "views" in result, f"cap-driver has no prompter op: {result}"
    (view,) = result["views"]
    assert view.get("overview"), f"no overview screen from the teleprompter: {view}"
    assert view["recording"] is False, "the overview is before recording"
    assert result["marks"] == [] and file_snapshot(tmp_path) == before
    screen = view["overview"]
    for row in screen["rows"]:
        missing = ROW - row.keys()
        assert not missing, f"overview row lacks {sorted(missing)}: {row}"
    return screen


@pytest.mark.parametrize("limit,over", [(90, False), (60, True)])
def test_overview_lists_every_shot_of_the_full_list_and_the_total(
    driver, tmp_path, limit, over
):
    whole = full()
    pickup = {"shots": whole["shots"][1:]}
    screen = overview(driver, tmp_path, whole, pickup, limit)
    assert [
        (r["shot_id"], r["one_liner"], r["performer_label"], r["budget_s"])
        for r in screen["rows"]
    ] == [
        (s["shot_id"], s["title"], s["performer_label"], s["budget_s"])
        for s in whole["shots"]
    ], "the overview is not full.json's shots in order"
    assert screen["total_s"] == 75 and screen["limit_s"] == limit
    assert screen["over_limit"] is over
    assert "75" in screen["summary"] and str(limit) in screen["summary"]


def test_a_0_5_0_list_shows_estimated_budgets_no_actions_and_presenter(
    driver, tmp_path
):
    legacy = listed("intro", "demo")
    legacy["shots"][1]["lines"].append(
        {"sentence_id": "demo-3", "text": "One two three four five six seven."}
    )
    validate_contract("shotlist", legacy)
    screen = overview(driver, tmp_path, legacy, legacy, 90)
    # intro: 6 spoken words -> 3 s; demo: 13 words -> 6 s; no actions in 0.5.0.
    estimates = [math.ceil(6 / 2.5), math.ceil(13 / 2.5)]
    rows = screen["rows"]
    assert [r["shot_id"] for r in rows] == ["intro", "demo"]
    assert [r["budget_label"] for r in rows] == [
        f"~{n} s (estimated)" for n in estimates
    ]
    for row in rows:
        assert row["performer_label"] == "Presenter"
        assert row.get("actions", []) == [], "a 0.5.0 list shows no actions"
    assert screen["total_s"] == sum(estimates)
