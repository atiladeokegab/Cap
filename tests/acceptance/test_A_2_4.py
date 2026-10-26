import json
import time

from conftest import digest, file_snapshot, finish_take


def test_bounded_stop_freezes_every_input_and_restart_needs_no_originals(mirror):
    finish_take(mirror.take)
    before = file_snapshot(mirror.take)
    mirror.server.put_budget = 3
    started = time.monotonic()
    result = mirror("stop", stopped_at_unix_ms=1793016000000, drain_timeout_ms=100)
    assert time.monotonic() - started < 3, (
        "The injected drain bound must not wait for the network"
    )
    assert result["frozen"] is True and result["drained"] is False
    accepted_before_restart = mirror.server.puts()
    assert len(accepted_before_restart) == 3
    assert accepted_before_restart[0][0].endswith("/stop.json")
    for relative, body in before.items():
        assert (mirror.outbox / relative).read_bytes() == body, relative
    assert file_snapshot(mirror.take) == before
    for path in mirror.take.rglob("*"):
        if path.is_file():
            path.unlink()
    mirror.server.put_budget = None
    mirror("resume")
    first_puts = mirror.server.puts()
    assert first_puts[0][0].endswith("/stop.json")
    stop = json.loads(first_puts[0][1])
    assert stop["stopped_at_unix_ms"] == 1793016000000
    assert set(stop["expected_files"]) == set(before)
    assert first_puts[-1][0].endswith("/take-manifest.json")
    assert (mirror.outbox / "uploaded.jsonl").is_file()
    manifest = json.loads(first_puts[-1][1])
    assert {item["path"] for item in manifest["files"]} == set(before)
    prefix = "/acceptance-fixture/takes/fixture/"
    uploaded = {path.removeprefix(prefix): body for path, body in first_puts}
    assert set(uploaded) == set(before) | {"stop.json", "take-manifest.json"}
    assert len(first_puts) == len(uploaded), "Restart re-PUT an already accepted object"
    assert first_puts[:3] == accepted_before_restart
    for relative, body in before.items():
        assert uploaded[relative] == body
    for item in manifest["files"]:
        assert item["sha256"] == digest(before[item["path"]])
    mirror("resume")
    assert mirror.server.puts() == first_puts, (
        "A new adapter process must resume without duplicate objects"
    )
