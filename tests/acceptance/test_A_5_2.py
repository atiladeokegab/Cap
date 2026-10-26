import json
import os
import shlex
import shutil
import struct

from conftest import ROOT, create_take, run, validate_contract


def test_cap_events_reads_binary_keyboard_and_preserves_segment_times(tmp_path):
    take = create_take(tmp_path / "fixture.cap")
    first = take / "content/segments/segment-0"
    second = take / "content/segments/segment-1"
    shutil.copytree(first, second)
    metadata_path = take / "recording-meta.json"
    metadata = json.loads(metadata_path.read_text())
    metadata["segments"].append(
        json.loads(
            json.dumps(metadata["segments"][0]).replace("segment-0", "segment-1")
        )
    )
    metadata_path.write_text(json.dumps(metadata))
    cursor_path = second / "cursor.json"
    cursor = json.loads(cursor_path.read_text())
    for event in cursor["moves"] + cursor["clicks"]:
        event["time_ms"] += 1000
    cursor_path.write_text(json.dumps(cursor))
    (second / "keyboard.bin").write_bytes(
        b"CPKB01\x01\x01a\x04KeyA" + struct.pack("<d", 1275.0) + b"\x01"
    )
    cap = shlex.split(os.environ.get("CAP_CLI", "target/debug/cap"))
    assert (ROOT / cap[0]).is_file(), "Build cap CLI or set CAP_CLI to its argv"
    output = json.loads(run([*cap, "events", str(take), "--json"]))
    events = output
    assert isinstance(events, list)
    by_kind = {
        kind: [e for e in events if e["type"] == kind]
        for kind in ("move", "click", "key")
    }
    assert len(by_kind["move"]) == len(by_kind["click"]) == len(by_kind["key"]) == 2
    for kind, expected in (("move", 250.0), ("click", 260.0), ("key", 275.0)):
        assert {(event["segment"], event["time_ms"]) for event in by_kind[kind]} == {
            (0, expected),
            (1, expected + 1000),
        }
    assert by_kind["key"][0]["key"] == "a"
    assert by_kind["key"][0]["key_code"] == "KeyA"
    validate_contract("cap-events", output)
