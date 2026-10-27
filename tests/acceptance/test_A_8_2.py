import json
import math
from itertools import pairwise

import pytest
from conftest import ROOT, validate_contract

FIXTURES = ROOT / "tests/acceptance/fixtures/zoom"
# Every bound of the line, in seconds unless named otherwise.
MIN_DWELL, IDLE_OUT, MIN_GAP, MAX_SHARE = 1.5, 2.0, 6.0, 0.35
EASE, DEPTH = (0.6, 0.9), (1.4, 2.0)
TOLERANCE = 0.1


def fixture(name):
    return json.loads((FIXTURES / f"{name}.json").read_text())


def zoom(driver, recording, **fields):
    """The zoom track the app generates, in seconds; Cap's cursor.json moves
    and clicks go in as recorded, keys as key-down times."""
    result = driver(
        "zoom",
        moves=recording["cursor"]["moves"],
        clicks=recording["cursor"]["clicks"],
        keys=[
            {"time_ms": k["time_ms"]}
            for k in recording["keyboard"]["presses"]
            if k["down"]
        ],
        duration_ms=recording["duration_s"] * 1000,
        **fields,
    )
    assert "segments" in result, f"cap-driver has no zoom op: {result}"
    return [
        {
            "start": s["start_ms"] / 1000,
            "end": s["end_ms"] / 1000,
            "amount": s["amount"],
            "ease": s["ease_ms"] / 1000,
        }
        for s in result["segments"]
    ]


def stock(take):
    """Cap's click-per-zoom generator, `generate_zoom_segments_from_clicks` in
    editor_edits.rs: every click pads to (t-0.3 s, t+2.5 s), the last second's
    clicks are ignored, intervals under 2.5 s apart merge, depth 2.0x."""
    duration_ms = take["duration_s"] * 1000
    intervals = sorted(
        (max(t - 300, 1), min(t + 2500, duration_ms - 800))
        for t in (math.floor(c["time_ms"]) for c in take["cursor"]["clicks"])
        if t < duration_ms - 1000
    )
    merged = []
    for start, end in (i for i in intervals if i[1] > i[0]):
        if merged and start <= merged[-1][1] + 2500:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return [
        {"start": round(s) / 1000, "end": round(e) / 1000, "amount": 2.0}
        for s, e in merged
    ]


def overlaps(segment, start, end):
    return segment["start"] < end and segment["end"] > start


def broken_bounds(take, segments):
    """The line's bounds a zoom track breaks on an annotated fixture."""
    broken = []
    starts = sorted(s["start"] for s in segments)
    if any(b - a < MIN_GAP - TOLERANCE for a, b in pairwise(starts)):
        broken.append("zoom-ins under 6 s apart")
    zoomed = sum(s["end"] - s["start"] for s in segments)
    if zoomed > MAX_SHARE * take["duration_s"] + TOLERANCE:
        broken.append(f"zoomed {zoomed:.1f} s of {take['duration_s']} s")
    for s in segments:
        if not DEPTH[0] <= s["amount"] <= DEPTH[1]:
            broken.append(f"depth {s['amount']}")
        if not 0 <= s["start"] < s["end"] <= take["duration_s"]:
            broken.append(f"segment outside the recording: {s}")
        work = [
            w for w in take["work"] if overlaps(s, w["from_s"] - EASE[1], w["to_s"])
        ]
        if not work:
            broken.append(f"zoom with no work under it: {s}")
        elif s["end"] > max(w["to_s"] for w in work) + IDLE_OUT + TOLERANCE:
            broken.append(f"held over 2 s past the work: {s}")
        broken += [
            f"zoomed on {d['why']}: {s}"
            for d in take["decoys"]
            if overlaps(s, d["from_s"], d["to_s"])
        ]
    return broken


def assert_eases(segments):
    for s in segments:
        assert EASE[0] <= s["ease"] <= EASE[1], f"ease of {s}"


@pytest.mark.parametrize("name", ["fast_clicks", "typing", "real_take"])
def test_recorded_fixture_stays_in_bounds_and_stock_breaks_them(driver, name):
    take = fixture(name)
    assert broken_bounds(take, stock(take)), "stock generator must break a bound"
    segments = zoom(driver, take)
    assert broken_bounds(take, segments) == []
    assert_eases(segments)
    # Following the work, not avoiding it: each fixture's work gets a zoom.
    assert any(
        overlaps(s, w["from_s"], w["to_s"]) for s in segments for w in take["work"]
    ), "no zoom on the fixture's work"


# Scenarios, built here so each clause has its own red.
class Take:
    def __init__(self, duration_s=30.0):
        self.take = {
            "duration_s": duration_s,
            "cursor": {"moves": [], "clicks": []},
            "keyboard": {"presses": []},
            "work": [],
            "decoys": [],
        }

    def click(self, t, x, y):
        self.take["cursor"]["moves"].append(
            {
                "active_modifiers": [],
                "cursor_id": "0",
                "time_ms": t * 1000 - 30,
                "x": x,
                "y": y,
            }
        )
        for dt, down in ((0, True), (0.07, False)):
            self.take["cursor"]["clicks"].append(
                {
                    "active_modifiers": [],
                    "cursor_num": 1,
                    "cursor_id": "0",
                    "time_ms": (t + dt) * 1000,
                    "down": down,
                }
            )
        return self

    def clicks(self, start, end, every, points):
        t, i = start, 0
        while t <= end + 1e-9:
            self.click(t, *points[i % len(points)])
            t, i = t + every, i + 1
        return self

    def keys(self, times):
        for t in times:
            for dt, down in ((0, True), (0.06, False)):
                self.take["keyboard"]["presses"].append(
                    {
                        "key": "a",
                        "key_code": "KeyA",
                        "time_ms": (t + dt) * 1000,
                        "down": down,
                    }
                )
        return self


def box(x, y, size):
    return [(x, y), (x + size, y + size), (x + size, y), (x, y + size)]


def test_zooms_in_on_work_that_stays_put_and_holds_while_it_does(driver):
    # Typing in one field 5-11.6 s with pauses under 2 s: one zoom, held.
    times = [5 + 0.15 * i for i in range(10)] + [7.0 + 0.15 * i for i in range(10)]
    times += [9.9 + 0.15 * i for i in range(12)]
    take = Take().click(4.8, 0.40, 0.30).keys(times).take
    segments = zoom(driver, take)
    assert len(segments) == 1, segments
    (s,) = segments
    assert 4.8 - EASE[1] <= s["start"] <= 4.8 + MIN_DWELL + TOLERANCE
    assert s["end"] >= times[-1], "zoomed out while the typing went on"
    assert s["end"] <= times[-1] + IDLE_OUT + TOLERANCE, "held after 2 s idle"
    assert_eases(segments)


def test_work_under_1_5_s_makes_no_zoom(driver):
    take = Take().clicks(10.0, 11.0, 0.25, box(0.5, 0.5, 0.05)).take
    assert zoom(driver, take) == []


@pytest.mark.parametrize(
    "points",
    [
        [(0.1, 0.1), (0.9, 0.9), (0.1, 0.9), (0.9, 0.1)],  # whole screen
        [(0.15, 0.15), (0.85, 0.85), (0.15, 0.85), (0.85, 0.15)],  # 49% of it
    ],
)
def test_work_spread_over_40_percent_of_the_frame_makes_no_zoom(driver, points):
    assert zoom(driver, Take().clicks(5.0, 9.0, 0.3, points).take) == []


def test_isolated_clicks_make_no_zoom(driver):
    take = Take().click(6.0, 0.3, 0.4).click(14.0, 0.31, 0.41).click(22.0, 0.7, 0.6)
    assert zoom(driver, take.take) == []


@pytest.mark.parametrize("y", [0.012, 0.985], ids=["title_bar", "taskbar"])
def test_clicks_on_window_chrome_make_no_zoom(driver, y):
    take = Take().clicks(5.0, 8.0, 0.4, [(0.955, y), (0.975, y), (0.993, y)]).take
    assert zoom(driver, take) == []


def test_a_drag_across_the_screen_makes_no_zoom(driver):
    take = Take()
    moves = take.take["cursor"]["moves"]
    for i in range(41):
        moves.append(
            {
                "active_modifiers": [],
                "cursor_id": "0",
                "time_ms": 5000 + 50 * i,
                "x": 0.1 + 0.02 * i,
                "y": 0.5,
            }
        )
    for t, down in ((5000, True), (7000, False)):
        take.take["cursor"]["clicks"].append(
            {
                "active_modifiers": [],
                "cursor_num": 1,
                "cursor_id": "0",
                "time_ms": t,
                "down": down,
            }
        )
    assert zoom(driver, take.take) == []


def test_zooms_out_when_the_work_moves_away_or_stops_for_2_s(driver):
    take = Take(40.0).clicks(5.0, 9.8, 0.4, box(0.2, 0.2, 0.06))
    take.click(10.0, 0.85, 0.85).click(10.4, 0.1, 0.9)  # the work moves away
    take.clicks(20.0, 24.0, 0.4, box(0.6, 0.3, 0.06))  # then stops at 24 s
    take.click(27.0, 0.6, 0.3).click(27.3, 0.62, 0.32)  # too short to zoom again
    segments = sorted(zoom(driver, take.take), key=lambda s: s["start"])
    assert len(segments) == 2, segments
    first, second = segments
    assert 9.8 <= first["end"] <= 10.0 + 0.3, "did not zoom out as the work moved"
    assert 24.0 <= second["end"] <= 24.0 + IDLE_OUT + TOLERANCE


def test_zoom_ins_are_6_s_apart_and_zoomed_time_is_at_most_35_percent(driver):
    # A qualifying 2.5 s burst every 4 s, each far from the one before.
    take = Take(44.0)
    for i, start in enumerate(range(2, 42, 4)):
        take.clicks(start, start + 2.5, 0.3, box(0.1 + 0.65 * (i % 2), 0.4, 0.04))
    segments = zoom(driver, take.take)
    starts = sorted(s["start"] for s in segments)
    assert len(starts) >= 2, "zoom stopped following the work"
    assert all(b - a >= MIN_GAP for a, b in pairwise(starts)), starts
    assert sum(s["end"] - s["start"] for s in segments) <= MAX_SHARE * 44.0


def test_depth_is_1_4_to_2_0_and_tighter_on_a_smaller_region(driver):
    take = Take(40.0).clicks(5.0, 9.0, 0.3, box(0.30, 0.30, 0.04))
    take.clicks(22.0, 26.0, 0.3, box(0.25, 0.25, 0.50))
    segments = sorted(zoom(driver, take.take), key=lambda s: s["start"])
    assert len(segments) == 2, segments
    small, large = segments
    for s in segments:
        assert DEPTH[0] <= s["amount"] <= DEPTH[1], s
    assert small["amount"] > large["amount"]
    assert_eases(segments)


def test_a_shot_marked_zoom_none_gets_no_zoom(driver, tmp_path):
    take = Take(60.0)
    for start in (5.0, 25.0, 45.0):
        take.clicks(start, start + 4.0, 0.3, box(0.3, 0.3, 0.05))
    shots = [
        {"shot_id": "intro", "title": "Intro", "lines": []},
        {"shot_id": "demo", "title": "Demo", "lines": [], "zoom": "none"},
        {"shot_id": "ask", "title": "Ask", "lines": []},
    ]
    marks = [
        {"shot_id": sid, "segment": 0, "time_ms": ms}
        for sid, ms in (("intro", 0), ("demo", 20000), ("ask", 40000))
    ]
    validate_contract("shotlist", {"shots": shots})
    validate_contract("shots", marks)
    plain = [{k: v for k, v in shot.items() if k != "zoom"} for shot in shots]
    control = zoom(driver, take.take, shotlist={"shots": plain}, marks=marks)
    assert any(overlaps(s, 20.0, 40.0) for s in control), "control: demo zooms"
    segments = zoom(driver, take.take, shotlist={"shots": shots}, marks=marks)
    assert not any(overlaps(s, 20.0, 40.0) for s in segments), segments
    assert any(overlaps(s, 0.0, 20.0) for s in segments)
    assert any(overlaps(s, 40.0, 60.0) for s in segments)


def test_a_take_keeps_its_zoom_none_after_pickups_are_republished(driver, tmp_path):
    # The take is marked against a list whose demo shot says zoom: none; the
    # film then republishes pick-ups without it. The editor resolves the
    # opt-out from the list recorded with the take, not the live one.
    take = Take(60.0)
    for start in (5.0, 25.0, 45.0):
        take.clicks(start, start + 4.0, 0.3, box(0.3, 0.3, 0.05))
    recorded = {
        "shots": [
            {"shot_id": "intro", "title": "Intro", "lines": []},
            {"shot_id": "demo", "title": "Demo", "lines": [], "zoom": "none"},
            {"shot_id": "ask", "title": "Ask", "lines": []},
        ]
    }
    pickups = {"shots": [{"shot_id": "demo", "title": "Demo", "lines": []}]}
    driver(
        "shots",
        shotlist=recorded,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(tmp_path)},
            {"at_ms": 0, "action": "next"},
            {"at_ms": 20000, "action": "next"},
            {"at_ms": 40000, "action": "next"},
        ],
    )
    segments = zoom(driver, take.take, take=str(tmp_path), shotlist=pickups)
    assert not any(overlaps(s, 20.0, 40.0) for s in segments), segments
    assert any(overlaps(s, 40.0, 60.0) for s in segments)
    # Control: without the recorded list the live pick-ups decide, and demo zooms.
    (tmp_path / "shotlist.json").unlink()
    control = zoom(driver, take.take, take=str(tmp_path), shotlist=pickups)
    assert any(overlaps(s, 20.0, 40.0) for s in control), control


def marked_take(driver, take_dir, shotlist):
    driver(
        "shots",
        shotlist=shotlist,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(take_dir)},
            {"at_ms": 0, "action": "next"},
            {"at_ms": 20000, "action": "next"},
            {"at_ms": 40000, "action": "next"},
        ],
    )


RECORDED = {
    "shots": [
        {"shot_id": "intro", "title": "Intro", "lines": []},
        {"shot_id": "demo", "title": "Demo", "lines": [], "zoom": "none"},
        {"shot_id": "ask", "title": "Ask", "lines": []},
    ]
}
PICKUPS = {"shots": [{"shot_id": "demo", "title": "Demo", "lines": []}]}


def test_a_retake_after_pickups_keeps_the_take_list_it_was_marked_against(
    driver, tmp_path
):
    # Reopen a marked take once pick-ups have dropped zoom: none, then press
    # Retake: the take's list is written once, at its first start, never again.
    take = Take(60.0)
    for start in (5.0, 25.0, 45.0):
        take.clicks(start, start + 4.0, 0.3, box(0.3, 0.3, 0.05))
    marked_take(driver, tmp_path, RECORDED)
    snapshot = (tmp_path / "shotlist.json").read_bytes()
    driver(
        "shots",
        shotlist=PICKUPS,
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 50000, "action": "retake"}],
    )
    assert (tmp_path / "shotlist.json").read_bytes() == snapshot
    segments = zoom(driver, take.take, take=str(tmp_path), shotlist=PICKUPS)
    assert not any(overlaps(s, 20.0, 40.0) for s in segments), segments


@pytest.mark.parametrize("live", [PICKUPS, RECORDED, None])
def test_a_corrupt_take_list_is_refused_never_replaced_by_the_live_one(
    driver, tmp_path, live
):
    # A take list that is there but unreadable is an error, unlike a take from
    # before takes kept their list: no zoom is generated on a guessed list.
    take = Take(60.0)
    for start in (5.0, 25.0, 45.0):
        take.clicks(start, start + 4.0, 0.3, box(0.3, 0.3, 0.05))
    marked_take(driver, tmp_path, RECORDED)
    (tmp_path / "shotlist.json").write_text('{"shots":')
    with pytest.raises(AssertionError, match="shotlist.json"):
        zoom(driver, take.take, take=str(tmp_path), shotlist=live)
