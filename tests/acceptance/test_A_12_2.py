from conftest import file_snapshot, validate_contract

DONE = "All shots done: press Stop"


def shot(sid, title, performer):
    # 0.6.0 fields; validate once vendored.
    return {
        "shot_id": sid,
        "title": title,
        "lines": [{"sentence_id": f"{sid}-1", "text": f"{title} now."}],
        "budget_s": 10,
        "actions": [],
        "performer_label": performer,
    }


def planned(count=3):
    return {
        "shots": [
            shot("intro", "Say what it is", "Speaker 1"),
            shot("demo", "Show the export", "Speaker 2"),
            shot("ask", "Ask for the vote", "Presenter"),
        ][:count]
    }


def ticks(start, end, every=100):
    return [{"at_ms": t, "action": "tick"} for t in range(start, end, every)]


def prompter(driver, script, listed, take):
    result = driver(
        "prompter", script=script, shotlist=listed, take=str(take), epoch_ms=0
    )
    assert "views" in result, f"cap-driver has no prompter op: {result}"
    assert len(result["views"]) == len(script), "one view per script step"
    for view in result["views"]:
        missing = {"phase", "recording", "card", "chime", "state", "state_color"}
        missing |= {"countdown", "countdown_label", "message"}
        missing -= view.keys()
        assert not missing, f"prompter view lacks {sorted(missing)}: {view}"
    return result


def index(script, action, at_ms):
    return next(
        i for i, s in enumerate(script) if s["action"] == action and s["at_ms"] == at_ms
    )


def test_next_while_running_shows_a_full_paused_card_and_chimes_once_until_go(
    driver, tmp_path
):
    script = [
        {"at_ms": 100, "action": "go"},
        *ticks(200, 4000),
        {"at_ms": 4000, "action": "next"},  # Ctrl+Alt+N while Running
        *ticks(4100, 9000, 300),
        {"at_ms": 9000, "action": "go"},
        *ticks(9100, 14000),
    ]
    result = prompter(driver, script, planned(), tmp_path)
    views = result["views"]
    pressed, went = index(script, "next", 4000), index(script, "go", 9000)
    running = views[pressed - 1]
    assert running["phase"] == "running" and running["recording"] is True

    for view in views[pressed:went]:
        assert view["phase"] == "reading" and view["recording"] is False, view
        card = view["card"]
        assert card is not None, "the next-shot card left before Go"
        assert card["heading"] == "Shot 2 of 3: Show the export", card
        assert card["performer_label"] == "Speaker 2", card
        assert card["full_window"] is True, "the card does not fill the teleprompter"
        assert view["state"] == "PAUSED", view
        assert view["state_color"] != running["state_color"], (
            "PAUSED is not shown in a colour distinct from recording"
        )
    assert running["state"] != "PAUSED"

    chimes = [i for i, view in enumerate(views) if view["chime"]]
    assert chimes == [pressed], f"chime must play once, on the transition: {chimes}"

    counting = [v for v in views[went:] if v["phase"] == "countdown"]
    assert counting, "Go started no countdown"
    for view in counting:
        assert view["countdown_label"] == "Recording shot 2", view
    assert views[-1]["phase"] == "running" and views[-1]["recording"] is True
    assert [m["shot_id"] for m in result["marks"]] == ["intro", "demo"]


def test_next_in_reading_or_countdown_is_inert_no_skip_no_mark_no_chime(
    driver, tmp_path
):
    script = [
        {"at_ms": 100, "action": "go"},
        *ticks(200, 4000),
        {"at_ms": 4000, "action": "next"},  # Running -> Reading: the one chime
        *ticks(4100, 5000, 300),
        {"at_ms": 5000, "action": "next"},  # Reading: inert
        {"at_ms": 5200, "action": "next"},
        {"at_ms": 5400, "action": "next"},
        *ticks(5500, 6000, 300),
        {"at_ms": 6000, "action": "go"},
        *ticks(6100, 6500),
        {"at_ms": 6500, "action": "next"},  # countdown: inert
        *ticks(6600, 11000),
    ]
    result = prompter(driver, script, planned(), tmp_path)
    views = result["views"]
    for at_ms in (5000, 5200, 5400):
        view = views[index(script, "next", at_ms)]
        assert view["phase"] == "reading", view
        assert view["card"]["heading"] == "Shot 2 of 3: Show the export", (
            "repeated Next in Reading skipped a shot"
        )
        assert view["chime"] is False, "Next in Reading chimed again"
    during = views[index(script, "next", 6500)]
    assert during["phase"] == "countdown", during
    assert during["countdown_label"] == "Recording shot 2"
    assert during["chime"] is False, "Next in the countdown chimed"

    assert sum(1 for view in views if view["chime"]) == 1, "more than one chime"
    marks = result["marks"]
    assert [m["shot_id"] for m in marks] == ["intro", "demo"], (
        f"Next in Reading or countdown wrote or skipped a mark: {marks}"
    )
    validate_contract("shots", marks)


def test_the_last_shot_shows_all_shots_done_instead_of_a_next_card(driver, tmp_path):
    script = [
        {"at_ms": 100, "action": "go"},
        *ticks(200, 4000),
        {"at_ms": 4000, "action": "next"},
        {"at_ms": 4500, "action": "go"},
        *ticks(4600, 9000),
        {"at_ms": 9000, "action": "next"},  # Next on the last shot
        *ticks(9100, 10000, 300),
    ]
    result = prompter(driver, script, planned(2), tmp_path)
    views = result["views"]
    before_last = views[index(script, "next", 9000) - 1]
    assert before_last["phase"] == "running", "the last shot never ran"
    for view in views[index(script, "next", 9000) :]:
        assert view["message"] == DONE, view
        assert view["card"] is None, "the last shot showed a next card"
    assert [m["shot_id"] for m in result["marks"]] == ["intro", "demo"]


def test_reading_before_the_first_go_plays_no_chime(driver, tmp_path):
    before = file_snapshot(tmp_path)
    result = prompter(driver, ticks(100, 2000, 300), planned(), tmp_path)
    assert not any(view["chime"] for view in result["views"]), "chime with no shot"
    assert result["marks"] == [] and file_snapshot(tmp_path) == before
