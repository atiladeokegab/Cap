import json

import pytest
from conftest import Held, listed, one_line_warning, republished, source


def marked(take):
    return [mark["shot_id"] for mark in json.loads((take / "shots.json").read_text())]


def test_take_start_and_reload_read_the_list_again_and_show_its_first_shot(
    driver, xml_server, tmp_path
):
    reads, bucket = republished(
        xml_server,
        listed("intro", "demo", "ask"),
        listed("demo", "ask"),
        listed("ask", "demo"),
    )
    take = tmp_path / "take"
    take.mkdir()
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(take)},
            {"at_ms": 100, "action": "next"},
            {"at_ms": 200, "action": "retake"},
            {"at_ms": 300, "action": "next"},
            {"at_ms": 400, "action": "take_stop"},
            {"at_ms": 500, "action": "reload"},
        ],
    )
    # Window open, take start, Reload: never on Next, Retake or stop.
    assert len(reads) == 3
    assert marked(take) == ["demo", "demo", "ask"]
    assert result["current"]["shot_id"] == "ask"
    assert result["warning"] is None
    assert '"Reload"' in source("apps/desktop-gpui/src/teleprompter_window.rs")


@pytest.mark.parametrize(
    "republished_mid_take", [listed("demo", "intro"), listed("ask")]
)
def test_a_list_changed_mid_take_never_moves_the_shot_on_screen(
    driver, xml_server, tmp_path, republished_mid_take
):
    full = listed("intro", "demo", "ask")
    reads, bucket = republished(xml_server, full, full, republished_mid_take)
    take = tmp_path / "take"
    take.mkdir()
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(take)},
            {"at_ms": 100, "action": "next"},
            {"at_ms": 200, "action": "next"},
            {"at_ms": 300, "action": "reload"},
            {"at_ms": 400, "action": "retake"},
            {"at_ms": 500, "action": "reload"},
            {"at_ms": 600, "action": "retake"},
        ],
    )
    assert len(reads) == 4
    assert marked(take) == ["intro", "demo", "demo", "demo"]
    assert result["current"]["shot_id"] == "demo"
    assert result["current"]["lines"] == [
        line["text"] for line in full["shots"][1]["lines"]
    ]


@pytest.mark.parametrize(
    "bad_read",
    [
        None,
        b"not json{",
        b"[]",
        {"shots": []},
        {"shots": [{"shot_id": "Demo", "title": "Demo", "lines": []}]},
        {
            "shots": [
                {
                    "shot_id": "demo",
                    "title": "Demo",
                    "lines": [{"sentence_id": "x", "text": ""}],
                }
            ]
        },
        b'{"shots": [{"shot_id": "demo", "title": "\xff", "lines": []}]}',
        b"[" * 100_000 + b"]" * 100_000,
        # Untrusted size: the teleprompter refuses a list over 1 MiB.
        {**listed("demo"), "title": "x" * (2 << 20)},
    ],
    ids=[
        "404",
        "not-json",
        "not-an-object",
        "no-shots",
        "bad-shot-id",
        "empty-line",
        "not-utf8",
        "deeply-nested",
        "oversized",
    ],
)
def test_a_failed_or_invalid_read_keeps_the_last_good_list_with_one_warning_line(
    driver, xml_server, tmp_path, bad_read
):
    reads, bucket = republished(
        xml_server, listed("intro", "demo"), listed("demo"), bad_read
    )
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
            {"at_ms": 200, "action": "take_stop"},
            {"at_ms": 300, "action": "take_start", "take": str(second)},
            {"at_ms": 400, "action": "next"},
            {"at_ms": 500, "action": "take_stop"},
            {"at_ms": 600, "action": "reload"},
        ],
    )
    assert len(reads) == 4
    assert result["mode"] == "shots"
    assert marked(first) == marked(second) == ["demo"]
    assert result["current"]["shot_id"] == "demo"
    one_line_warning(result)


def test_unknown_fields_in_the_bucket_list_are_ignored(driver, xml_server, tmp_path):
    extended = listed("demo", "ask")
    extended["performer_default"] = "lead"
    extended["shots"][0].update(
        performer="agent", url="https://example.test", script="demo.py"
    )
    extended["shots"][0]["lines"][0]["voice"] = {"take": 3}
    reads, bucket = republished(xml_server, listed("intro"), extended)
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[{"at_ms": 0, "action": "take_start", "take": str(tmp_path)}],
    )
    assert len(reads) == 2
    assert result["current"]["shot_id"] == "demo" and result["warning"] is None


def test_a_late_read_never_replaces_the_list_a_take_started_with(
    driver, xml_server, tmp_path
):
    # Reload is pressed and its read hangs; the take starts and reads the newer
    # list; then the old read lands, after the take is under way.
    reads, bucket = republished(
        xml_server,
        listed("intro", "demo", "ask"),
        Held(listed("ask")),
        listed("demo", "intro"),
    )
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "reload"},
            {"at_ms": 100, "action": "take_start", "take": str(tmp_path)},
            {"at_ms": 200, "action": "next"},
            {"at_ms": 300, "action": "settle"},
            {"at_ms": 400, "action": "retake"},
            {"at_ms": 500, "action": "next"},
            {"at_ms": 600, "action": "take_stop"},
        ],
    )
    assert not reads.stalled, "The teleprompter waited on a slow read"
    assert len(reads) == 3
    assert marked(tmp_path) == ["demo", "demo", "intro"]
    # Between takes the last good list is the take's, not the late one.
    assert result["current"]["shot_id"] == "demo"


def test_a_late_read_never_replaces_a_newer_list_mid_take(driver, xml_server, tmp_path):
    full = listed("intro", "demo", "ask")
    reads, bucket = republished(
        xml_server, full, full, Held(listed("ask")), listed("demo", "intro")
    )
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(tmp_path)},
            {"at_ms": 100, "action": "next"},
            {"at_ms": 200, "action": "next"},
            {"at_ms": 300, "action": "reload"},
            {"at_ms": 400, "action": "reload"},
            {"at_ms": 500, "action": "settle"},
            {"at_ms": 600, "action": "retake"},
            {"at_ms": 700, "action": "take_stop"},
        ],
    )
    assert not reads.stalled, "The teleprompter waited on a slow read"
    assert len(reads) == 4
    assert marked(tmp_path) == ["intro", "demo", "demo"]
    assert result["current"]["shot_id"] == "demo"


def test_a_reload_before_the_first_mark_keeps_the_list_the_take_started_with(
    driver, xml_server, tmp_path
):
    # The take starts on the list read at its start; Reload is pressed before
    # its first mark, and the newer list waits for the take to end.
    reads, bucket = republished(
        xml_server, listed("intro"), listed("demo"), listed("ask")
    )
    take = tmp_path / "take"
    take.mkdir()
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[
            {"at_ms": 0, "action": "take_start", "take": str(take)},
            {"at_ms": 100, "action": "reload"},
        ],
    )
    assert len(reads) == 3
    assert result["current"]["shot_id"] == "demo", result
