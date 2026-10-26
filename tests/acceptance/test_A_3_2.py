import json

import pytest
from conftest import shotlist


@pytest.mark.parametrize("delta", [125.0, 900.25, 12345.5])
def test_next_shot_uses_start_gate_clock_not_ui_elapsed(driver, tmp_path, delta):
    result = driver(
        "shots",
        shotlist=shotlist(),
        take=str(tmp_path),
        epoch_ms=50000,
        elapsed_offset_ms=9000,
        script=[{"at_ms": 50000 + delta, "action": "next"}],
    )
    assert len(result["marks"]) == 1
    mark = result["marks"][0]
    assert mark["shot_id"] == "intro" and mark["segment"] == 0
    assert abs(mark["time_ms"] - delta) <= 5
    assert json.loads((tmp_path / "shots.json").read_text()) == result["marks"]
