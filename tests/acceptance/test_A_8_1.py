import json

from conftest import file_snapshot, validate_contract

SCRIPT_TEXT_PX = 30  # the teleprompter's default script size (store.rs)


def planned():
    return {
        "shots": [
            {
                "shot_id": "intro",
                "title": "Introduce it",
                "on_screen": "The product",
                "lines": [
                    {"sentence_id": f"i{n}", "text": f"Intro line {n}."}
                    for n in range(3)
                ],
            },
            {
                "shot_id": "demo",
                "title": "Show the export",
                "on_screen": "The editor with a take open",
                "lines": [{"sentence_id": "d0", "text": "Export it."}],
            },
        ]
    }


def prompter(driver, script, **fields):
    result = driver("prompter", script=script, **fields)
    assert "views" in result, f"cap-driver has no prompter op: {result}"
    assert len(result["views"]) == len(script), "one view per script step"
    return result


def ticks(start, end, every=100):
    return [{"at_ms": t, "action": "tick"} for t in range(start, end, every)]


# A-11.1: the teleprompter opens in Reading; Go, then the 3-2-1, starts the
# first shot. Each case begins this way; its own steps follow from SHIFT ms.
SHIFT = 4500
START = [{"at_ms": 100, "action": "go"}, *ticks(200, SHIFT)]
P = len(START) - 1  # the view at the end of START: the first shot running


def shifted(steps):
    return [{**step, "at_ms": step["at_ms"] + SHIFT} for step in steps]


def resumed_ms(script, views):
    """The step the first Go's countdown ended on: the new segment's epoch."""
    first = [v["recording"] for v in views].index(True)
    return script[first]["at_ms"]


def test_stop_cue_next_shot_card_and_go_countdown_resume_new_segment(driver, tmp_path):
    validate_contract("shotlist", planned())
    script = START + shifted(
        [
            {"at_ms": 900, "action": "go"},
            {"at_ms": 1500, "action": "scroll", "line": 1},
            {"at_ms": 2500, "action": "scroll", "line": 2},
            {"at_ms": 3500, "action": "scroll", "line": 3},
            {"at_ms": 4000, "action": "next"},
            {"at_ms": 6000, "action": "go"},
            *ticks(6100, 10500),
        ]
    )
    result = prompter(
        driver, script, shotlist=planned(), take=str(tmp_path), epoch_ms=0
    )
    views, marks = result["views"], result["marks"]
    assert result["mode"] == "shots"

    started, idle_go, line1, last_line, past = views[P : P + 5]
    assert started["shot_id"] == "intro" and started["recording"] is True
    assert started["card"] is None and started["cue"] is None
    # Go with no card waiting does nothing.
    assert idle_go["recording"] is True and idle_go["countdown"] is None
    assert line1["cue"] is None and last_line["cue"] is None, "cue before the end"
    assert past["cue"] == "stop", "no STOP cue past the shot's last line"

    card_view = views[P + 5]
    assert card_view["recording"] is False, "Next shot must pause the recording"
    card = card_view["card"]
    assert card and card["go"] is True
    assert card["title"] == "Show the export"
    assert card["on_screen"] == "The editor with a take open"
    assert card["text_px"] >= SCRIPT_TEXT_PX + 6, "card text is not large"

    after_go = views[P + 6 :]
    recording = [v["recording"] for v in after_go]
    assert True in recording, "Go never resumed the recording"
    resume = recording.index(True)
    shown = [v["countdown"] for v in after_go[:resume]]
    assert [n for i, n in enumerate(shown) if i == 0 or shown[i - 1] != n] == [
        3,
        2,
        1,
    ], f"countdown before the resume: {shown}"
    for n in (3, 2, 1):
        assert shown.count(n) >= 5, f"{n} shown under 0.5 s"
    assert all(v["countdown"] is None for v in after_go[resume:])
    went = SHIFT + 6000
    assert went + 2400 <= script[P + 6 + resume]["at_ms"] <= went + 4000
    resumed = views[-1]
    assert resumed["recording"] is True and resumed["card"] is None
    assert resumed["countdown"] is None and resumed["cue"] is None
    assert resumed["shot_id"] == "demo" and resumed["line"] == 0

    # The mark lands at the resume, on the new segment's own clock (A-3.2).
    assert [m["shot_id"] for m in marks] == ["intro", "demo"]
    assert marks[1]["segment"] == marks[0]["segment"] + 1, "no new segment"
    assert marks[0]["time_ms"] <= 5, "first shot's mark not written at the resume"
    assert marks[1]["time_ms"] <= 5, "shot mark not written at the resume"
    assert json.loads((tmp_path / "shots.json").read_text()) == marks
    validate_contract("shots", marks)


def test_retake_rewinds_to_the_shots_first_line(driver, tmp_path):
    script = START + shifted(
        [
            {"at_ms": 1500, "action": "scroll", "line": 2},
            {"at_ms": 2500, "action": "scroll", "line": 3},
            {"at_ms": 3000, "action": "retake"},
        ]
    )
    result = prompter(
        driver, script, shotlist=planned(), take=str(tmp_path), epoch_ms=0
    )
    assert result["views"][P]["recording"] is True, "the first shot never ran"
    past, rewound = result["views"][P + 2 :]
    assert past["cue"] == "stop"
    assert rewound["shot_id"] == "intro" and rewound["line"] == 0
    assert rewound["cue"] is None and rewound["card"] is None
    # A retake still appends its mark (A-3.3).
    last = result["marks"][-1]
    first = result["marks"][0]
    assert (last["shot_id"], last["segment"], last.get("retake")) == (
        "intro",
        first["segment"],
        True,
    )
    # On the segment's own clock, which starts at the resume (A-3.2).
    resumed = resumed_ms(script, result["views"])
    assert abs(last["time_ms"] - (SHIFT + 3000 - resumed)) <= 5


def test_free_mode_shows_no_cue(driver, tmp_path):
    before = file_snapshot(tmp_path)
    script = [
        {"at_ms": 500, "action": "scroll", "line": 40},
        {"at_ms": 900, "action": "next"},
        {"at_ms": 1000, "action": "go"},
        *ticks(1100, 4500, 500),
        {"at_ms": 4600, "action": "retake"},
    ]
    result = prompter(driver, script, shotlist=None, take=str(tmp_path), epoch_ms=0)
    assert result["mode"] == "free" and result["marks"] == []
    for view in result["views"]:
        assert view["cue"] is None and view["card"] is None, view
        assert view["countdown"] is None and view["recording"] is True, view
    assert file_snapshot(tmp_path) == before
