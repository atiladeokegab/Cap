import json

from conftest import listed, republished, validate_contract


def test_pickup_take_marks_carry_the_pickup_shots_own_ids_on_its_own_clock(
    driver, xml_server, tmp_path
):
    full = listed("intro", "demo", "ask")
    # Only demo and ask, and in each only its second line, in script order.
    pickup = {
        "shots": [
            {**shot, "lines": shot["lines"][1:]}
            for shot in full["shots"]
            if shot["shot_id"] != "intro"
        ]
    }
    validate_contract("shotlist", pickup)
    _, bucket = republished(xml_server, full, full, pickup)
    first, second = tmp_path / "first", tmp_path / "second"
    first.mkdir()
    second.mkdir()
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(first)},
            {"at_ms": 100, "action": "next"},
            {"at_ms": 200, "action": "next"},
            {"at_ms": 300, "action": "next"},
            {"at_ms": 400, "action": "take_stop"},
            # The pick-up take's start gate arms at 5000: its marks count from there.
            {"at_ms": 5000, "action": "take_start", "take": str(second)},
            {"at_ms": 5100, "action": "next"},
            {"at_ms": 5250, "action": "retake"},
            {"at_ms": 5400, "action": "next"},
        ],
    )
    whole = json.loads((first / "shots.json").read_text())
    marks = json.loads((second / "shots.json").read_text())
    validate_contract("shots", marks)
    assert [m["shot_id"] for m in whole] == ["intro", "demo", "ask"]
    assert [m["shot_id"] for m in marks] == ["demo", "demo", "ask"]
    assert [bool(m.get("retake")) for m in marks] == [False, True, False]
    assert [m["segment"] for m in marks] == [0, 0, 0]
    for mark, expected in zip(marks, [100, 250, 400]):
        assert abs(mark["time_ms"] - expected) <= 5
    assert result["current"]["shot_id"] == "ask"
    assert result["current"]["lines"] == [pickup["shots"][1]["lines"][0]["text"]]
