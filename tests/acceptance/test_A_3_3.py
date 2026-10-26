import pytest
from conftest import shotlist


def test_retake_appends_and_resumed_mark_uses_new_epoch(driver, tmp_path):
    result = driver(
        "shots",
        shotlist=shotlist(),
        take=str(tmp_path),
        epoch_ms=0,
        elapsed_offset_ms=9000,
        script=[
            {"at_ms": 1000, "action": "next"},
            {"at_ms": 2000, "action": "retake"},
            {"at_ms": 2200, "action": "pause"},
            {"at_ms": 5000, "action": "resume"},
            {"at_ms": 5150, "action": "retake"},
        ],
    )
    marks = result["marks"]
    assert len(marks) == 3 and [mark["shot_id"] for mark in marks] == ["intro"] * 3
    assert [mark["segment"] for mark in marks] == [0, 0, 1]
    for mark, expected in zip(marks, [1000, 2000, 150]):
        assert abs(mark["time_ms"] - expected) <= 5


@pytest.mark.parametrize("action,expected", [("retake", "demo"), ("next", "ask")])
def test_reopened_teleprompter_restores_current_shot_from_saved_marks(
    driver, tmp_path, action, expected
):
    planned = {
        "shots": [
            {
                "shot_id": sid,
                "title": sid,
                "lines": [{"sentence_id": f"s{index}", "text": sid}],
            }
            for index, sid in enumerate(("intro", "demo", "ask"), 1)
        ]
    }
    first = driver(
        "shots",
        shotlist=planned,
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 100, "action": "next"}, {"at_ms": 200, "action": "next"}],
    )
    assert [mark["shot_id"] for mark in first["marks"]] == ["intro", "demo"]
    reopened = driver(
        "shots",
        shotlist=planned,
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 300, "action": action}],
    )
    assert reopened["marks"][:-1] == first["marks"]
    assert reopened["marks"][-1]["shot_id"] == expected
    assert reopened["current"]["shot_id"] == expected
