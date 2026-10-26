import ctypes
import json
import os
import shutil
import subprocess
import time
from pathlib import Path

from conftest import display_seconds, media_seconds, record_screen

FRAME = 1 / 60
# Cap samples the pointer on a timer: a move lands within one poll of happening.
POLL = 0.1
FORK_FILES = ("shots.json", "windows.jsonl", "stop.json", "take-manifest.json")
INGEST = (
    "import json,sys; from film.ingest import acceptance; "
    "print(json.dumps(acceptance(json.load(sys.stdin), {})))"
)


def warp(display, x, y):
    """Moves the real X pointer, the way xdotool does."""
    x11 = ctypes.CDLL("libX11.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XDefaultRootWindow.restype = ctypes.c_ulong
    x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    x11.XWarpPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong] + [
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_uint,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_int,
    ]
    x11.XFlush.argtypes = x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    connection = x11.XOpenDisplay(display.encode())
    assert connection, f"Cannot open X display {display}"
    try:
        x11.XWarpPointer(
            connection, 0, x11.XDefaultRootWindow(connection), 0, 0, 0, 0, x, y
        )
        x11.XFlush(connection)
    finally:
        x11.XCloseDisplay(connection)


def ingest(film, request):
    """The film's ingest, through its own acceptance seam, in its own checkout."""
    result = subprocess.run(
        [
            "uv",
            "run",
            "--offline",
            "--no-project",
            "--with",
            "jsonschema",
            "python",
            "-c",
            INGEST,
        ],
        cwd=film,
        input=json.dumps(request),
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout.splitlines()[-1])


def test_agent_take_is_a_stock_take_the_film_ingests_with_its_pointer(
    xdisplay, tmp_path
):
    film = Path(os.environ.get("FILM_REPO", Path.home() / "code/film"))
    assert (film / "film/ingest/__init__.py").is_file(), (
        f"Set FILM_REPO to a film checkout with film.ingest (the watcher's ingest), not {film}"
    )
    targets = [(480, 270), (1440, 810)]
    moved = []

    def move(clock):
        time.sleep(0.5)
        for x, y in targets:
            before = time.monotonic()
            warp(xdisplay, x, y)
            moved.append((x, y, before, time.monotonic()))
            time.sleep(0.5)

    take = tmp_path / "agent-take.cap"
    clock = record_screen(xdisplay, take, 3.0, during=move)
    assert not [name for name in FORK_FILES if (take / name).exists()], (
        "Not a stock take"
    )
    meta = json.loads((take / "recording-meta.json").read_text())
    assert (meta.get("status") or {}).get("status") in (None, "Complete", "NeedsRemux")
    (segment,) = meta["segments"]
    assert segment.get("mic") is None, "The agent take has no mic track"

    moves = json.loads((take / segment["cursor"]).read_text())["moves"]
    for x, y, before, after in moved:
        hits = [
            m["time_ms"] / 1000
            for m in moves
            if abs(m["x"] - x / 1920) < 0.01 and abs(m["y"] - y / 1080) < 0.01
        ]
        assert hits, f"Pointer moved to ({x}, {y}) is not in cursor.json"
        # On the recording's clock: zero lies between start called and start returned.
        assert (
            before - clock["started"] <= hits[0] <= after - clock["start_called"] + POLL
        )

    bucket = tmp_path / "bucket/takes/agent-take"
    shutil.copytree(take, bucket)
    project = ingest(
        film,
        {
            "op": "project",
            "bucket_dir": str(bucket),
            "output_dir": str(tmp_path / "derived"),
        },
    )
    display = Path(project["tracks"]["display"])
    assert abs(media_seconds(display) - display_seconds(take, tmp_path)) <= FRAME
    stock = ingest(
        film,
        {
            "op": "stock",
            "take": project["project"],
            "output_dir": str(tmp_path / "stock"),
        },
    )
    assert stock["accepted"] is True and "stock" in json.dumps(stock["warnings"])
    assert stock["shots"] == [] and stock["windows"] == []
