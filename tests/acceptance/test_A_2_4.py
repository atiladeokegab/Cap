import json
import time

import pytest
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
    expected = {item["path"]: item for item in stop["expected_files"]}
    assert set(expected) == set(before)
    for relative, body in before.items():
        assert expected[relative]["size"] == len(body)
        assert expected[relative]["sha256"] == digest(body)
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


@pytest.mark.parametrize("defect", ["missing", "corrupted", "outside-outbox"])
def test_resume_never_certifies_missing_corrupt_or_escaping_inventory(mirror, defect):
    finish_take(mirror.take)
    mirror.server.fail_puts = True
    stopped = mirror("stop", stopped_at_unix_ms=1793016000000, drain_timeout_ms=100)
    assert stopped["frozen"] and not stopped["drained"]
    stop_path = mirror.outbox / "stop.json"
    stop = json.loads(stop_path.read_text())
    if defect == "missing":
        (mirror.outbox / "project-config.json").unlink()
    elif defect == "corrupted":
        (mirror.outbox / "project-config.json").write_bytes(b"corrupted-after-freeze")
    else:
        marker = b"synthetic-outside-proof"
        (mirror.outbox.parent / "outside-proof").write_bytes(marker)
        stop["expected_files"].append(
            {"path": "../outside-proof", "size": len(marker), "sha256": digest(marker)}
        )
        stop_path.write_text(json.dumps(stop))
    mirror.server.fail_puts = False
    try:
        result = mirror("resume", drain_timeout_ms=100)
    except AssertionError as error:
        assert defect == "outside-outbox" and any(
            word in str(error).lower() for word in ("path", "outbox", "parent")
        ), str(error)
    else:
        assert not result["drained"], "Invalid inventory was reported complete"
    assert not any(
        path.endswith("/take-manifest.json") for path, _ in mirror.server.puts()
    )
    if defect == "outside-outbox":
        assert not any(body == marker for _, body in mirror.server.puts()), (
            "Resume read outside its outbox"
        )
