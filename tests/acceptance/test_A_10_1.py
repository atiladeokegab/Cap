import json

from conftest import display_seconds, record_screen

FRAME = 1 / 60


def test_release_cli_records_headless_with_no_ld_library_path(xdisplay, tmp_path):
    take = tmp_path / "agent-take.cap"
    clock = record_screen(xdisplay, take, 3.0)
    meta = json.loads((take / "recording-meta.json").read_text())
    (segment,) = meta["segments"]
    assert segment["display"]["fps"] == 60
    seconds = display_seconds(take, tmp_path)
    # The track covers the whole time between start returning and stop being
    # called, and no more than start called to stop returned, within a frame.
    held = clock["stop_called"] - clock["started"]
    spanned = clock["stopped"] - clock["start_called"]
    assert held - FRAME <= seconds <= spanned + FRAME, (held, seconds, spanned)
