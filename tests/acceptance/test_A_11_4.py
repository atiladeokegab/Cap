import json

import pytest
from conftest import shotlist, validate_contract

BASE_MS = 1000  # the retake key's events start after the first shot's mark


def down(t, repeat=False):
    return {"t": t, "edge": "down", "repeat": repeat}


def up(t):
    return {"t": t, "edge": "up", "repeat": False}


HELD = [down(0.0), *(down(0.5 * n, repeat=True) for n in range(1, 7)), up(3.1)]
FIXTURES = {
    # Held 3 s: the OS auto-repeats from 0.5 s to 3.0 s.
    "held_with_repeats": (HELD, [0.0], [0.5, 1.0, 1.5, 2.0, 2.5, 3.0]),
    "two_presses_1_s_apart": (
        [down(0.0), up(0.1), down(1.0), up(1.1)],
        [0.0, 1.0],
        [],
    ),
    "two_presses_0_3_s_apart": (
        [down(0.0), up(0.1), down(0.3), up(0.4)],
        [0.0],
        [0.3],
    ),
    # Exactly 0.5 s after the last accepted press is accepted; a rejected
    # press (0.3) must not reset the cooldown.
    "exact_0_5_s_after_a_rejected_press": (
        [down(0.0), up(0.1), down(0.3), up(0.4), down(0.5), up(0.6)],
        [0.0, 0.5],
        [0.3],
    ),
    # A repeat with no flag: a second key-down with no key-up since the first.
    "down_without_up_between": (
        [down(0.0), down(0.8), up(0.9)],
        [0.0],
        [0.8],
    ),
}


@pytest.mark.parametrize("name", list(FIXTURES))
def test_retake_marks_only_on_a_fresh_press_at_least_0_5_s_apart(
    driver, tmp_path, name
):
    events, marked, ignored = FIXTURES[name]
    script = [{"at_ms": 0, "action": "next"}] + [
        {
            "at_ms": BASE_MS + round(e["t"] * 1000),
            "action": "retake_key",
            "edge": e["edge"],
            "repeat": e["repeat"],
        }
        for e in events
    ]
    result = driver(
        "shots", shotlist=shotlist(), take=str(tmp_path), epoch_ms=0, script=script
    )
    retakes = [m for m in result["marks"] if m.get("retake")]
    assert len(retakes) == len(marked), f"{name}: retake marks {retakes}"
    for mark, t in zip(retakes, marked):
        assert abs(mark["time_ms"] - (BASE_MS + t * 1000)) <= 5, mark
    assert json.loads((tmp_path / "shots.json").read_text()) == result["marks"]
    validate_contract("shots", result["marks"])

    log = result.get("ignored_keys")
    assert isinstance(log, list), "ignored retake key-downs are not logged"
    assert [entry["at_ms"] for entry in log] == [
        BASE_MS + round(t * 1000) for t in ignored
    ], f"{name}: ignored key-downs logged {log}"
    for entry in log:
        assert isinstance(entry.get("reason"), str) and entry["reason"].strip(), (
            f"an ignored key-down is logged without its reason: {entry}"
        )
