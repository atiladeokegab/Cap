from conftest import source, validate_contract


def test_windows_emit_only_changes_on_recording_epoch(driver):
    result = driver(
        "windows",
        epoch_ms=50000,
        elapsed_offset_ms=9000,
        samples=[
            {"at_ms": 50100, "title": "Démo", "process": "demo.exe"},
            {"at_ms": 50350, "title": "Démo", "process": "demo.exe"},
            {"at_ms": 50600, "title": "Result", "process": "demo.exe"},
            {"at_ms": 50850, "title": "Result", "process": "other.exe"},
        ],
    )
    lines = result["lines"]
    assert [line["time_ms"] for line in lines] == [100, 600, 850]
    assert [line["title"] for line in lines] == ["Démo", "Result", "Result"]
    assert [line["process"] for line in lines] == ["demo.exe", "demo.exe", "other.exe"]
    assert all(line["segment"] == 0 for line in lines)
    for line in lines:
        validate_contract("windows", line)
    windows = source("apps/desktop-gpui/src/window_log.rs")
    assert "cfg(windows)" in windows or 'target_os = "windows"' in windows
