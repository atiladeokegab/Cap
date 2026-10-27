import sys

import pytest

WINDOW = {"background_alpha", "text_alpha", "click_through", "capture_excluded"}


def shotlist():
    return {
        "shots": [
            {
                "shot_id": sid,
                "title": sid.title(),
                "lines": [{"sentence_id": f"{sid}-1", "text": f"{sid} line."}],
            }
            for sid in ("intro", "demo")
        ]
    }


def ticks(start, end, every=100):
    return [{"at_ms": t, "action": "tick"} for t in range(start, end, every)]


def prompter(driver, script, tmp_path):
    """The teleprompter on Linux, through the driver: each view reports the
    window flags it requests from the platform layer."""
    result = driver(
        "prompter",
        script=script,
        shotlist=shotlist(),
        take=str(tmp_path),
        app_data=str(tmp_path / "app-data"),
        epoch_ms=0,
    )
    assert "views" in result, f"cap-driver has no prompter op: {result}"
    assert len(result["views"]) == len(script), "one view per script step"
    for view in result["views"]:
        window = view.get("window")
        assert isinstance(window, dict), f"prompter view reports no window: {view}"
        missing = WINDOW - window.keys()
        assert not missing, f"window flags lack {sorted(missing)}: {view}"
    return result["views"]


def run_then(*steps, until=8000):
    """Reading, Go, the countdown, then Running with `steps` from 5000 ms on."""
    return [
        {"at_ms": 50, "action": "tick"},  # Reading, before Go
        {"at_ms": 100, "action": "go"},
        *ticks(200, 5000),
        *steps,
        *ticks(5000 + 100 * len(steps) + 100, until),
    ]


def running(views):
    return [v for v in views if v["phase"] == "running"]


def test_running_is_translucent_click_through_and_never_captured(driver, tmp_path):
    views = prompter(driver, run_then(), tmp_path)
    reading = [v for v in views if v["phase"] == "reading"]
    assert reading and running(views), "Reading then Running expected"
    for view in running(views):
        window = view["window"]
        assert window["background_alpha"] == pytest.approx(0.40), window
        assert window["text_alpha"] == 1.0, "text is not at full contrast"
        assert window["click_through"] is True, "Running takes the clicks"
    for view in reading:
        window = view["window"]
        assert window["background_alpha"] == 1.0, "a Reading card is not opaque"
        assert window["click_through"] is False, "a Reading card ignores clicks"
    for view in views:
        assert view["window"]["capture_excluded"] is True, (
            f"the teleprompter can be captured: {view}"
        )


def test_mouse_over_fades_to_15_percent_and_back(driver, tmp_path):
    script = run_then(
        {"at_ms": 5000, "action": "mouse", "over": True},
        {"at_ms": 5100, "action": "mouse", "over": False},
    )
    views = prompter(driver, script, tmp_path)
    entered = next(i for i, s in enumerate(script) if s["action"] == "mouse")
    over, away = views[entered], views[entered + 1]
    assert over["phase"] == "running", over
    assert over["window"]["background_alpha"] == pytest.approx(0.15), over
    assert over["window"]["click_through"] is True
    assert away["window"]["background_alpha"] == pytest.approx(0.40), away


@pytest.mark.parametrize("percent,alpha", [(70, 0.70), (10, 0.20), (95, 0.90)])
def test_opacity_is_adjustable_20_to_90_and_remembered(
    driver, tmp_path, percent, alpha
):
    script = run_then({"at_ms": 5000, "action": "opacity", "percent": percent})
    views = prompter(driver, script, tmp_path)
    assert running(views)[-1]["window"]["background_alpha"] == pytest.approx(alpha)
    # A new teleprompter, same app data: the setting was kept.
    again = prompter(driver, run_then(), tmp_path)
    assert running(again)[-1]["window"]["background_alpha"] == pytest.approx(alpha)


def test_next_shot_reading_card_is_opaque_and_takes_clicks(driver, tmp_path):
    script = run_then({"at_ms": 5000, "action": "next"}, until=6000)
    views = prompter(driver, script, tmp_path)
    card = views[-1]
    assert card["phase"] == "reading", card
    assert card["window"]["background_alpha"] == 1.0, "the next card is translucent"
    assert card["window"]["click_through"] is False
    assert card["window"]["capture_excluded"] is True


# Behavioural proof of the Windows helpers in platform/windows.rs: the driver
# opens a real teleprompter window through them and reports what Windows reads
# back, not what was requested. It needs a real Windows desktop; on Linux the
# state-model tests above cover the requested flags.
@pytest.mark.skipif(sys.platform != "win32", reason="needs a real Windows window")
def test_on_windows_the_real_window_applies_the_flags(driver):
    result = driver("teleprompter_window", phases=["running", "reading"])
    assert result.get("backend") == "windows", result
    run, read = result["running"], result["reading"]
    # GetLayeredWindowAttributes / GetWindowLongPtrW(GWL_EXSTYLE) read-backs.
    assert run["alpha_read_back"] == pytest.approx(0.40, abs=0.01), run
    assert read["alpha_read_back"] == pytest.approx(1.0, abs=0.01), read
    assert run["ex_style_transparent"] is True, "Running is not click-through"
    assert read["ex_style_transparent"] is False, "Reading does not take clicks"
    # WindowFromPoint at the window's centre: the app underneath, or the prompter.
    assert run["hit_test_passes_through"] is True, run
    assert read["hit_test_passes_through"] is False, read
    for flags in (run, read):
        assert flags["display_affinity"] == "WDA_EXCLUDEFROMCAPTURE", flags
