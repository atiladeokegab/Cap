from conftest import file_snapshot


def test_free_mode_writes_no_shot_file_or_take_changes(driver, tmp_path):
    before = file_snapshot(tmp_path)
    result = driver(
        "shots",
        shotlist=None,
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 1000, "action": "next"}, {"at_ms": 1100, "action": "retake"}],
    )
    assert result["mode"] == "free" and result["marks"] == []
    assert file_snapshot(tmp_path) == before
