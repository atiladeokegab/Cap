import json
import time
from datetime import UTC, datetime, timedelta

import pytest
from conftest import create_take, digest, file_snapshot, finish_take, write_json

TITLE = "Send this take to the film?"
# 0.7.0 send.json; validate once vendored.
SEND_KEYS = {"send", "decided_at", "cap_send_version"}


@pytest.fixture
def studio(driver, xml_server, tmp_path):
    """A stopped take in the app's layout (recordings/<take>/outbox), mirrored
    to the fake bucket, its stop drained so take-manifest.json has landed."""
    recordings = tmp_path / "recordings"
    take = create_take(recordings / "take.cap")
    write_json(
        take / "shots.json",
        [
            {"shot_id": "intro", "segment": 0, "time_ms": 0.0},
            {"shot_id": "demo", "segment": 0, "time_ms": 1500.0},
        ],
    )
    finish_take(take)
    config = tmp_path / "config.json"
    write_json(
        config,
        {
            "bucket": "acceptance-fixture",
            "access_key": "acceptance-access",
            "secret_key": "acceptance-only-secret",
            "region": "auto",
            "prefix": "takes/fixture",
        },
    )

    def invoke(op, **fields):
        return driver(
            op,
            endpoint=xml_server.endpoint,
            config=str(config),
            take=str(take),
            outbox=str(take / "outbox"),
            recordings=str(recordings),
            **fields,
        )

    stopped = invoke("stop", stopped_at_unix_ms=1793016000000, drain_timeout_ms=10000)
    assert stopped["frozen"] and stopped["drained"], stopped
    invoke.take, invoke.root, invoke.server = take, tmp_path, xml_server
    invoke.cap_files = cap_files(take)
    invoke.bucket = dict(xml_server.objects)
    return invoke


def cap_files(take):
    return {
        path: body
        for path, body in file_snapshot(take).items()
        if not path.startswith("outbox/")
    }


def dialog(studio, choice, opened_from="stop"):
    result = studio("send_dialog", choice=choice, opened_from=opened_from)
    assert "dialog" in result, f"cap-driver has no send_dialog op: {result}"
    screen = result["dialog"]
    assert screen["title"] == TITLE, screen
    assert {"Send", "Keep back"} <= set(screen["buttons"]), screen
    # shots.json: intro at 0 s, demo at 1.5 s, in a 4 s segment.
    rows = [(row["shot_id"], row["length_s"]) for row in screen["shots"]]
    assert [sid for sid, _ in rows] == ["intro", "demo"], rows
    assert [length for _, length in rows] == pytest.approx([1.5, 2.5], abs=0.1)
    return result


def restart(studio):
    """A new Cap process: resume_pending over the recordings folder."""
    result = studio("resume_pending")
    assert "resumed" in result, f"cap-driver has no resume_pending op: {result}"
    return result


def bucket_send(studio):
    sent = [path for path in studio.server.objects if path.endswith("/send.json")]
    assert len(sent) <= 1, f"more than one send.json object: {sent}"
    if not sent:
        return None, None
    return sent[0], json.loads(studio.server.objects[sent[0]])


def send_puts(studio):
    return [
        json.loads(body)
        for path, body in studio.server.puts()
        if path.endswith("/send.json")
    ]


def decided(record, after, before):
    assert SEND_KEYS <= record.keys(), f"send.json lacks {SEND_KEYS - record.keys()}"
    assert isinstance(record["cap_send_version"], str) and record["cap_send_version"]
    at = datetime.fromisoformat(record["decided_at"])
    assert at.utcoffset() == timedelta(0), "decided_at is not UTC"
    slack = timedelta(seconds=1)
    window = (
        datetime.fromtimestamp(after, UTC) - slack,
        datetime.fromtimestamp(before, UTC) + slack,
    )
    assert window[0] <= at <= window[1], f"decided_at {at} is not the wall clock"
    return record["decided_at"]


def assert_capture_untouched(studio):
    """Keep back gates the film only: the streamed objects, the manifest and
    every file Cap wrote stay byte for byte; send.json is never in a hash."""
    manifest_key = next(p for p in studio.bucket if p.endswith("/take-manifest.json"))
    manifest = json.loads(studio.bucket[manifest_key])
    folder = manifest_key.removesuffix("take-manifest.json")
    listed = {item["path"]: item["sha256"] for item in manifest["files"]}
    assert not any(p.endswith("send.json") for p in listed), "manifest lists send.json"
    stop = json.loads(studio.bucket[folder + "stop.json"])
    assert not any(f["path"].endswith("send.json") for f in stop["expected_files"])
    for path, sha in listed.items():
        assert digest(studio.server.objects[folder + path]) == sha, path
    for path, body in studio.bucket.items():
        assert studio.server.objects[path] == body, f"{path} changed after Stop"
    after = cap_files(studio.take)
    for path, body in studio.cap_files.items():
        assert after.get(path) == body, f"{path}, written by Cap, changed"
    extra = {p for p in after if p not in studio.cap_files}
    assert all(p.rsplit("/", 1)[-1] == "send.json" for p in extra), extra
    return folder


def test_closing_the_dialog_decides_nothing(studio):
    before = file_snapshot(studio.root)
    result = dialog(studio, "close")
    assert result.get("send") is None, "closing the dialog wrote a decision"
    restart(studio)
    assert not list(studio.root.rglob("send.json")), "a send.json was written"
    assert file_snapshot(studio.root) == before
    assert bucket_send(studio) == (None, None), "send.json reached the bucket"
    assert_capture_untouched(studio)


def test_keep_back_then_restart_then_send_from_the_recordings_list(studio):
    started = time.time()
    kept = dialog(studio, "keep_back")["send"]
    assert kept["send"] is False, kept
    decided(kept, started, time.time())
    assert_capture_untouched(studio)

    restart(studio)
    _, record = bucket_send(studio)
    assert record is None or record["send"] is False, "Keep back sent the take"

    # Send's own upload fails: the worker is gone, so only the durable path remains.
    studio.server.fail_puts = True
    started = time.time()
    sent = dialog(studio, "send", opened_from="recordings_list")["send"]
    first = decided(sent, started, time.time())
    assert sent["send"] is True, sent
    studio.server.fail_puts = False
    restart(studio)
    path, record = bucket_send(studio)
    assert record is not None, "resume_pending did not upload the pending send.json"
    assert record["send"] is True and record["decided_at"] == first, record
    folder = assert_capture_untouched(studio)
    assert path == folder + "send.json", "send.json is not beside take-manifest.json"


def test_a_repeated_send_is_one_object_with_the_first_decided_at(studio):
    started = time.time()
    first = decided(dialog(studio, "send")["send"], started, time.time())
    restart(studio)
    time.sleep(1.2)  # a retry on a later second must not move decided_at
    again = dialog(studio, "send", opened_from="recordings_list")["send"]
    assert again["send"] is True and again["decided_at"] == first, again
    back = dialog(studio, "keep_back", opened_from="recordings_list")["send"]
    assert back["send"] is True and back["decided_at"] == first, (
        "send went true -> false; only false -> true is allowed"
    )
    restart(studio)
    restart(studio)
    _, record = bucket_send(studio)
    assert record is not None and record["send"] is True, record
    assert record["decided_at"] == first, "a retried Send moved decided_at"
    puts = send_puts(studio)
    assert len(puts) == 1, f"send.json was PUT {len(puts)} times"
    assert_capture_untouched(studio)
