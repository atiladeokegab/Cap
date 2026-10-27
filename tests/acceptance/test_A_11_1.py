import json
import math

import pytest
from conftest import file_snapshot, validate_contract

MAX_WPS = 2.5  # never scroll faster than this, in spoken words per second
PROMPT = "Read, then press Go (Enter) to start"
VIEW = {"phase", "recording", "card", "countdown", "cue", "scroll_words", "flash"}
CARD = {
    "title",
    "lines",
    "actions",
    "budget_s",
    "budget_label",
    "performer_label",
    "prompt",
    "over_budget",
}


def words(tag, n):
    return " ".join(f"{tag}{i}" for i in range(n)) + "."


def shot(sid, lines, budget_s, actions=(), performer="Presenter"):
    """A 0.6.0 shot: `lines` is how many words each line has."""
    return {
        "shot_id": sid,
        "title": sid.title(),
        "on_screen": f"The {sid} screen",
        "lines": [
            {"sentence_id": f"{sid}-{n}", "text": words(f"{sid}{n}w", count)}
            for n, count in enumerate(lines)
        ],
        "budget_s": budget_s,
        "actions": list(actions),
        "performer_label": performer,
    }


def planned():
    # 0.6.0 fields; validate once vendored.
    return {
        "shots": [
            # 30 words over 20 s: 1.5 words per second.
            shot("intro", [10, 10, 10], 20, ["Click Export"], "Speaker 1"),
            shot("demo", [6, 6], 10, ["Open the editor", "Drag the clip"]),
        ]
    }


def prompter(driver, script, shotlist, take):
    result = driver(
        "prompter", script=script, shotlist=shotlist, take=str(take), epoch_ms=0
    )
    assert "views" in result, f"cap-driver has no prompter op: {result}"
    assert len(result["views"]) == len(script), "one view per script step"
    for view in result["views"]:
        missing = VIEW - view.keys()
        assert not missing, f"prompter view lacks {sorted(missing)}: {view}"
        if view["card"] is not None:
            missing = CARD - view["card"].keys()
            assert not missing, f"Reading card lacks {sorted(missing)}: {view}"
    return result


def ticks(start, end, every=100):
    return [{"at_ms": t, "action": "tick"} for t in range(start, end, every)]


def key(at_ms, name):
    return {"at_ms": at_ms, "action": "key", "key": name}


def resumed_at(views):
    recording = [v["recording"] for v in views]
    assert True in recording, "Go never resumed the recording"
    return recording.index(True)


def rate(script, views, start, end):
    """Words per second scrolled between two views."""
    seconds = (script[end]["at_ms"] - script[start]["at_ms"]) / 1000
    return (views[end]["scroll_words"] - views[start]["scroll_words"]) / seconds


def test_reading_shows_the_whole_shot_and_waits_with_the_recording_paused(
    driver, tmp_path
):
    listed = planned()
    before = file_snapshot(tmp_path)
    script = ticks(100, 4000, 300)
    result = prompter(driver, script, listed, tmp_path)
    for view in result["views"]:
        assert view["phase"] == "reading", view
        assert view["recording"] is False, "Reading must keep the recording paused"
        assert view["countdown"] is None and view["cue"] is None, view
        assert view["scroll_words"] == 0, "the text scrolled by itself in Reading"
    assert result["marks"] == [], "Reading wrote a shot mark"
    assert file_snapshot(tmp_path) == before

    card = result["views"][-1]["card"]
    intro = listed["shots"][0]
    assert card["title"] == intro["title"]
    assert [line["text"] for line in card["lines"]] == [
        line["text"] for line in intro["lines"]
    ], "the card does not show the shot's full lines"
    assert [action["text"] for action in card["actions"]] == intro["actions"]
    line_styles = {line["style"] for line in card["lines"]}
    action_styles = {action["style"] for action in card["actions"]}
    assert not line_styles & action_styles, "actions are not styled apart from lines"
    assert card["budget_s"] == 20 and "20" in card["budget_label"]
    assert card["performer_label"] == "Speaker 1"
    assert card["prompt"] == PROMPT
    assert card["over_budget"] is False


def test_a_long_shot_is_read_by_hand_with_no_mark_written(driver, tmp_path):
    # 0.6.0 fields; validate once vendored.
    listed = {"shots": [shot("long", [12] * 40, 200)]}
    before = file_snapshot(tmp_path)
    script = [
        *ticks(100, 1000, 300),
        key(1000, "ArrowDown"),
        *ticks(1100, 2000, 300),
        key(2000, "ArrowDown"),
        {"at_ms": 2500, "action": "wheel", "delta_lines": 5},
        *ticks(2600, 3500, 300),
        key(3500, "ArrowUp"),
        *ticks(3600, 4500, 300),
    ]
    result = prompter(driver, script, listed, tmp_path)
    views = result["views"]
    at = {step["at_ms"]: view["scroll_words"] for step, view in zip(script, views)}
    assert at[100] == at[700] == 0, "the text scrolled before any key"
    assert at[1000] > 0, "ArrowDown did not scroll the long shot"
    assert at[1100] == at[1700] == at[1000], "the text scrolled by itself"
    assert at[2000] > at[1000], "ArrowDown did not scroll further"
    assert at[2500] > at[2000], "the mouse wheel did not scroll"
    assert at[2600] == at[3200] == at[2500], "the text scrolled by itself"
    assert at[3500] < at[2500], "ArrowUp did not scroll back"
    for view in views:
        assert view["phase"] == "reading" and view["recording"] is False, view
        assert view["countdown"] is None, "a scroll key started the countdown"
    assert result["marks"] == [], "reading by hand wrote a mark"
    assert file_snapshot(tmp_path) == before


@pytest.mark.parametrize(
    "go",
    [
        {"action": "key", "key": "Enter"},
        {"action": "key", "key": "Space"},
        {"action": "go"},
    ],
    ids=["enter", "space", "go_control"],
)
def test_go_counts_down_then_resumes_marks_and_only_then_scrolls(driver, tmp_path, go):
    script = [*ticks(100, 1000, 300), {"at_ms": 1000, **go}, *ticks(1100, 9100)]
    result = prompter(driver, script, planned(), tmp_path)
    views = result["views"]
    after_go = views[3:]
    resume = resumed_at(after_go)
    counting = after_go[:resume]
    assert counting, "the recording resumed with no countdown"
    for view in counting:
        assert view["phase"] == "countdown" and view["recording"] is False, view
        assert view["scroll_words"] == 0, "the text scrolled before the resume"
    shown = [v["countdown"] for v in counting]
    assert [n for i, n in enumerate(shown) if i == 0 or shown[i - 1] != n] == [
        3,
        2,
        1,
    ], f"countdown before the resume: {shown}"
    resume_ms = script[3 + resume]["at_ms"]
    assert 1000 + 2400 <= resume_ms <= 1000 + 4000

    running = after_go[resume:]
    for view in running:
        assert view["phase"] == "running" and view["recording"] is True, view
        assert view["countdown"] is None and view["card"] is None, view
    # The start-gate mark lands at the resume, on the new segment's clock (A-3.2).
    marks = result["marks"]
    assert [m["shot_id"] for m in marks] == ["intro"]
    assert marks[0]["time_ms"] <= 5, "shot mark not written at the resume"
    assert json.loads((tmp_path / "shots.json").read_text()) == marks
    validate_contract("shots", marks)
    # 30 words over a 20 s budget: 1.5 words per second.
    first = 3 + resume
    assert abs(rate(script, views, first, len(views) - 1) - 1.5) <= 0.15


def test_a_shot_over_its_budget_scrolls_at_2_5_words_per_second_and_warns(
    driver, tmp_path
):
    # 0.6.0 fields; validate once vendored.
    # 50 words in 10 s would be 5 words per second.
    listed = {"shots": [shot("rushed", [25, 25], 10)]}
    script = [*ticks(50, 100), {"at_ms": 100, "action": "go"}, *ticks(200, 10200)]
    result = prompter(driver, script, listed, tmp_path)
    views = result["views"]
    card = views[0]["card"]
    assert card is not None and card["over_budget"] is True, "no over-budget warning"
    first = resumed_at(views)
    assert abs(rate(script, views, first, len(views) - 1) - MAX_WPS) <= 0.15
    for a in range(first, len(views) - 10, 10):
        assert rate(script, views, a, a + 10) <= MAX_WPS + 0.05, "over 2.5 w/s"


def test_next_while_running_stops_pauses_and_opens_the_next_reading_card(
    driver, tmp_path
):
    script = [
        {"at_ms": 100, "action": "go"},
        *ticks(200, 5000, 200),
        {"at_ms": 5000, "action": "next"},  # Ctrl+Alt+N
        *ticks(5100, 7000, 300),
        {"at_ms": 7000, "action": "go"},
        *ticks(7100, 11100),
    ]
    result = prompter(driver, script, planned(), tmp_path)
    views = result["views"]
    pressed = [s["action"] for s in script].index("next")
    assert views[pressed - 1]["phase"] == "running", "intro never ran"
    stop = views[pressed]
    assert stop["cue"] == "stop", "Ctrl+Alt+N while Running shows no STOP cue"
    assert stop["recording"] is False, "Ctrl+Alt+N did not pause the recording"
    assert stop["phase"] == "reading"
    assert stop["card"] is not None and stop["card"]["title"] == "Demo"
    assert stop["card"]["prompt"] == PROMPT
    went = [s["action"] for s in script].index("go", pressed)
    for view in views[pressed:went]:
        assert view["phase"] == "reading" and view["recording"] is False, view
        assert view["scroll_words"] == 0, "the next shot scrolled before Go"

    marks = result["marks"]
    assert [m["shot_id"] for m in marks] == ["intro", "demo"]
    assert marks[1]["segment"] == marks[0]["segment"] + 1, "no new segment"
    assert marks[1]["time_ms"] <= 5, "demo's mark not written at the resume"
    assert views[-1]["phase"] == "running" and views[-1]["recording"] is True
    validate_contract("shots", marks)


def test_next_in_reading_only_flashes_press_go(driver, tmp_path):
    before = file_snapshot(tmp_path)
    script = [
        *ticks(100, 500, 200),
        {"at_ms": 500, "action": "next"},  # Ctrl+Alt+N
        *ticks(600, 1000, 200),
        {"at_ms": 1000, "action": "next"},
    ]
    result = prompter(driver, script, planned(), tmp_path)
    views = result["views"]
    quiet = views[0]
    assert quiet["flash"] is None
    for index in (2, 5):
        pressed = views[index]
        assert pressed["flash"] == "Press Go to start", pressed
    for view in views:
        assert view["phase"] == "reading" and view["recording"] is False, view
        assert view["countdown"] is None and view["cue"] is None, view
        assert view["card"]["title"] == "Intro", "Ctrl+Alt+N in Reading moved on"
        assert view["scroll_words"] == 0
    assert result["marks"] == [], "Ctrl+Alt+N in Reading wrote a mark"
    assert file_snapshot(tmp_path) == before


def test_a_0_5_0_list_loads_with_an_estimated_budget_and_presenter(driver, tmp_path):
    legacy = {
        "shots": [
            {
                "shot_id": "intro",
                "title": "Intro",
                "lines": [
                    {"sentence_id": "intro-0", "text": words("a", 7)},
                    {"sentence_id": "intro-1", "text": words("b", 7)},
                ],
            }
        ]
    }
    validate_contract("shotlist", legacy)
    result = prompter(driver, ticks(100, 400), legacy, tmp_path)
    card = result["views"][0]["card"]
    # 14 spoken words: ceil(14 / 2.5) = 6 s, no actions.
    assert card["budget_label"] == f"~{math.ceil(14 / MAX_WPS)} s (estimated)"
    assert card["actions"] == [], "a 0.5.0 list shows no actions"
    assert card["performer_label"] == "Presenter"
    assert card["prompt"] == PROMPT


def test_a_shot_without_budget_estimates_2_s_per_action(driver, tmp_path):
    # 0.6.0 fields; validate once vendored.
    partial = shot("demo", [7, 7], None, ["Open the editor", "Drag the clip"])
    del partial["budget_s"]
    result = prompter(driver, ticks(100, 400), {"shots": [partial]}, tmp_path)
    card = result["views"][0]["card"]
    estimate = math.ceil(14 / MAX_WPS) + 2 * 2
    assert card["budget_label"] == f"~{estimate} s (estimated)"
