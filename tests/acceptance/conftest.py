import hashlib
import hmac
import json
import os
import re
import shlex
import shutil
import struct
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, quote, unquote, urlsplit

import pytest

ROOT = Path(__file__).resolve().parents[2]


def run(command, *, timeout=180, env=None, input=None):
    result = subprocess.run(
        command,
        cwd=ROOT,
        env={**os.environ, **(env or {})},
        input=input,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return result.stdout


def source(path):
    target = ROOT / path
    assert target.is_file(), f"Required implementation missing: {path}"
    return target.read_text()


def validate_contract(name, value, version=None):
    if version is None:
        assert (ROOT / "contracts/vendor").is_dir(), (
            "Missing released vendored contracts"
        )
        releases = [
            p
            for p in (ROOT / "contracts/vendor").iterdir()
            if p.is_dir() and re.fullmatch(r"\d+\.\d+\.\d+", p.name)
        ]
        assert len(releases) == 1, "Choose the take's exact vendored contracts version"
        version = releases[0].name
    assert re.fullmatch(r"\d+\.\d+\.\d+", version)
    schema = ROOT / "contracts/vendor" / version / (name + ".schema.json")
    assert schema.is_file(), f"Missing released contract: {schema}"
    run(
        [
            "uv",
            "run",
            "--offline",
            "--no-project",
            "--with",
            "jsonschema",
            "python",
            "-c",
            "import json,sys; from jsonschema import validate; validate(json.load(sys.stdin),json.load(open(sys.argv[1])))",
            str(schema),
        ],
        input=json.dumps(value),
    )


@pytest.fixture
def driver():
    command = shlex.split(
        os.environ.get("CAP_ACCEPTANCE_DRIVER", "target/acceptance/cap-driver")
    )

    def invoke(op, **fields):
        assert command and (
            (ROOT / command[0]).is_file() or shutil.which(command[0])
        ), "Build the test-only mirror adapter or set CAP_ACCEPTANCE_DRIVER to its argv"
        output = run(command, input=json.dumps({"op": op, **fields}), timeout=20)
        result = json.loads(output)
        assert isinstance(result, dict), "Adapter returns one JSON object on stdout"
        return result

    return invoke


@pytest.fixture
def live():
    if os.environ.get("CAP_LIVE") != "1":
        pytest.skip("Live Windows/GCS/release evidence: opt in with CAP_LIVE=1")


def workflow_text():
    candidates = []
    for path in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        text = "\n".join(
            line
            for line in path.read_text().splitlines()
            if not line.lstrip().startswith("#")
        )
        if (
            re.search(r"['\"]?v\*['\"]?(?=\s|$)", text)
            and "workflow_dispatch" in text
            and "windows-2022" in text
        ):
            candidates.append((path, text))
    assert len(candidates) == 1, (
        f"Expected one fork release workflow, found {[str(p) for p, _ in candidates]}"
    )
    return candidates[0][1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def signature(method, url, headers, body, secret, scope, signed_headers):
    parts = urlsplit(url)
    names = signed_headers.split(";")
    normalized = {k.lower(): " ".join(v.split()) for k, v in headers.items()}
    canonical_headers = "".join(f"{k}:{normalized[k]}\n" for k in names)
    pairs = sorted(
        (quote(k, safe="-_.~"), quote(v, safe="-_.~"))
        for k, v in parse_qsl(parts.query, keep_blank_values=True)
    )
    query = "&".join(f"{k}={v}" for k, v in pairs)
    path = quote(unquote(parts.path or "/"), safe="/-_.~")
    canonical = "\n".join(
        [method, path, query, canonical_headers, signed_headers, digest(body)]
    )
    date = normalized["x-amz-date"]
    to_sign = "\n".join(["AWS4-HMAC-SHA256", date, scope, digest(canonical.encode())])
    key = ("AWS4" + secret).encode()
    for field in scope.split("/"):
        key = hmac.new(key, field.encode(), hashlib.sha256).digest()
    return hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()


class XMLStore:
    def __init__(self):
        self.objects = {}
        self.requests = []
        self.errors = []
        self.fail_puts = False
        self.put_budget = None

    def puts(self):
        return [
            (path, body)
            for method, path, body, status in self.requests
            if method == "PUT" and status == 200
        ]


@pytest.fixture
def xml_server():
    store = XMLStore()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def handle_request(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            path = unquote(urlsplit(self.path).path)
            status = 200
            payload = b""
            try:
                auth = self.headers.get("Authorization", "")
                match = re.fullmatch(
                    r"AWS4-HMAC-SHA256 Credential=([^/]+)/([^,]+),\s*SignedHeaders=([^,]+),\s*Signature=([a-f0-9]{64})",
                    auth,
                )
                assert match, "Missing or malformed SigV4 Authorization"
                access, scope, signed, actual = match.groups()
                assert access == "acceptance-access"
                expected = signature(
                    self.command,
                    self.path,
                    dict(self.headers),
                    body,
                    "acceptance-only-secret",
                    scope,
                    signed,
                )
                assert hmac.compare_digest(actual, expected), (
                    "Invalid request signature"
                )
                assert self.headers.get("x-amz-content-sha256") == digest(body)
            except (AssertionError, KeyError) as error:
                store.errors.append(str(error))
                status = 403
            if status == 200:
                if self.command == "PUT":
                    if store.fail_puts or store.put_budget == 0:
                        status = 503
                    else:
                        store.objects[path] = body
                        if store.put_budget is not None:
                            store.put_budget -= 1
                elif path in store.objects:
                    payload = store.objects[path]
                else:
                    status = 404
            store.requests.append((self.command, path, body, status))
            self.send_response(status)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header(
                "ETag",
                '"' + hashlib.md5(store.objects.get(path, b"")).hexdigest() + '"',
            )
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(payload)

        do_PUT = handle_request
        do_GET = handle_request
        do_HEAD = handle_request

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    store.endpoint = f"http://127.0.0.1:{server.server_port}"
    try:
        yield store
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
    assert not store.errors, store.errors


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


def file_snapshot(root):
    return {
        p.relative_to(root).as_posix(): p.read_bytes()
        for p in root.rglob("*")
        if p.is_file()
    }


def listed(*ids):
    """A shotlist.json holding these shots, each two lines long."""
    return {
        "shots": [
            {
                "shot_id": sid,
                "title": sid.title(),
                "lines": [
                    {"sentence_id": f"{sid}-{n}", "text": f"{sid.title()} line {n}."}
                    for n in (1, 2)
                ],
            }
            for sid in ids
        ]
    }


class Held:
    """A published version whose GET is answered only once a later GET of the
    key has arrived: an old, slow read the teleprompter must not wait on."""

    def __init__(self, value):
        self.value = value


def republished(store, *versions):
    """The film's shotlist/current.json, republished between reads: each GET
    gets the next version (a JSON value, raw bytes, None for a 404, or Held),
    the last one again once they run out. Returns the reads made (`stalled`
    lists held reads no later read overtook) and the driver fields that point
    the teleprompter at the key."""
    key = "/acceptance-fixture/shotlist/current.json"

    def encoded(v):
        if isinstance(v, Held):
            return Held(encoded(v.value))
        return v if v is None or isinstance(v, bytes) else json.dumps(v).encode()

    pending = [encoded(v) for v in versions]

    class Reads(list):
        pass

    reads, arrived, answer = Reads(), threading.Condition(), threading.local()
    reads.stalled = []

    class Objects(dict):
        def __contains__(self, path):
            if path != key:
                return super().__contains__(path)
            with arrived:
                version = pending.pop(0) if len(pending) > 1 else pending[0]
                reads.append(version)
                number = len(reads)
                arrived.notify_all()
            if isinstance(version, Held):
                with arrived:
                    if not arrived.wait_for(lambda: len(reads) > number, timeout=5):
                        reads.stalled.append(number)
                time.sleep(0.3)  # the later read is answered first
                version = version.value
            answer.body = version
            return version is not None

        def __getitem__(self, path):
            return answer.body if path == key else super().__getitem__(path)

    store.objects = Objects(store.objects)
    return reads, {
        "shotlist": None,
        "shotlist_url": store.endpoint + key,
        "access_key": "acceptance-access",
        "secret_key": "acceptance-only-secret",
    }


def one_line_warning(result):
    warning = result.get("warning")
    assert isinstance(warning, str) and warning.strip(), "Expected a warning line"
    assert "\n" not in warning.strip(), f"Warning is more than one line: {warning!r}"


@pytest.fixture
def xdisplay():
    """A fresh Xvfb display at 1920x1080 for the cap CLI to record."""
    if not shutil.which("Xvfb"):
        pytest.skip("Headless recording needs Xvfb (slow)")
    read, write = os.pipe()
    server = subprocess.Popen(
        [
            "Xvfb",
            "-displayfd",
            str(write),
            "-screen",
            "0",
            "1920x1080x24",
            "-nolisten",
            "tcp",
            "-noreset",
        ],
        pass_fds=[write],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    os.close(write)
    with os.fdopen(read) as announced:
        number = announced.readline().strip()
    try:
        assert number, "Xvfb did not start"
        yield ":" + number
    finally:
        server.terminate()
        server.wait(timeout=10)


def release_cli():
    cli = ROOT / os.environ.get("CAP_RELEASE_CLI", "target/release/cap")
    assert cli.is_file(), (
        "Build the release cap CLI (cargo build --release -p cap) or set CAP_RELEASE_CLI to it"
    )
    return cli


def record_screen(display, take, seconds, during=None):
    """`cap record start --detach`, then `cap record stop`, on `display`, with no
    LD_LIBRARY_PATH. `during(clock)` runs while recording. Returns monotonic
    times: start_called, started, stop_called, stopped."""
    env = {k: v for k, v in os.environ.items() if k != "LD_LIBRARY_PATH"}
    env["DISPLAY"] = display

    def cli(*args):
        result = subprocess.run(
            [str(release_cli()), *args],
            cwd=take.parent,
            env=env,
            capture_output=True,
            text=True,
            timeout=90,
            check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        return result.stdout

    screens = json.loads(cli("record", "screens", "--json"))
    full_hd = [
        s["id"]
        for s in screens
        if s["physicalSize"] == {"width": 1920.0, "height": 1080.0}
    ]
    assert full_hd, f"No 1920x1080 screen in {screens}"
    clock = {"start_called": time.monotonic()}
    cli("record", "start", "--screen", str(full_hd[0]), "--detach", "--path", str(take))
    clock["started"] = time.monotonic()
    stopped = False
    try:
        if during:
            during(clock)
        time.sleep(max(0.0, clock["started"] + seconds - time.monotonic()))
        clock["stop_called"] = time.monotonic()
        cli("record", "stop", "--path", str(take))
        stopped = True
        clock["stopped"] = time.monotonic()
    finally:
        if not stopped:
            subprocess.run(
                [str(release_cli()), "record", "stop", "--path", str(take)],
                env=env,
                capture_output=True,
                timeout=90,
                check=False,
            )
    return clock


def media_seconds(path):
    output = run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            str(path),
        ]
    )
    return float(output.strip())


def display_seconds(take, scratch):
    """The display track's length. A fragmented track (a folder) is joined,
    init then fragments in manifest order, into `scratch`, never the take."""
    meta = json.loads((take / "recording-meta.json").read_text())
    (segment,) = meta["segments"]
    track = take / segment["display"]["path"]
    if track.is_dir():
        manifest = json.loads((track / "manifest.json").read_text())
        # Cap's v5 m4s manifest lists "segments"; older ones list "fragments".
        entries = manifest.get("segments", manifest.get("fragments"))
        joined = scratch / "display-joined.mp4"
        parts = [manifest.get("init_segment", "init.mp4")] + [
            f["path"] for f in sorted(entries, key=lambda f: f["index"])
        ]
        joined.write_bytes(b"".join((track / part).read_bytes() for part in parts))
        track = joined
    return media_seconds(track)


def create_take(root):
    segment = root / "content/segments/segment-0"
    for track in ("display", "camera"):
        directory = segment / track
        directory.mkdir(parents=True)
        (directory / "init.mp4").write_bytes(b"fixture-init")
        (directory / "fragment_000.mp4").write_bytes(b"complete-" + track.encode())
        (directory / "fragment_001.mp4").write_bytes(b"in-progress")
        write_json(
            directory / "manifest.json",
            {
                "version": 2,
                "is_complete": False,
                "fragments": [
                    {
                        "path": "fragment_000.mp4",
                        "index": 0,
                        "duration": 2.0,
                        "is_complete": True,
                        "file_size": len(b"complete-" + track.encode()),
                    },
                    {
                        "path": "fragment_001.mp4",
                        "index": 1,
                        "duration": 2.0,
                        "is_complete": False,
                        "file_size": 11,
                    },
                ],
            },
        )
    write_json(
        root / "recording-meta.json",
        {
            "pretty_name": "Acceptance take",
            "sharing": None,
            "platform": "Windows",
            "segments": [
                {
                    "display": {
                        "path": "content/segments/segment-0/display",
                        "fps": 30,
                    },
                    "camera": {"path": "content/segments/segment-0/camera", "fps": 30},
                    "mic": {
                        "path": "content/segments/segment-0/audio-input.m4a",
                        "start_time": 0.3,
                    },
                    "system_audio": {
                        "path": "content/segments/segment-0/system_audio.m4a",
                        "start_time": 0.0,
                    },
                    "cursor": "content/segments/segment-0/cursor.json",
                    "keyboard": "content/segments/segment-0/keyboard.bin",
                }
            ],
            "cursors": {
                "0": {
                    "imagePath": "content/cursors/arrow.png",
                    "hotspot": {"x": 0, "y": 0},
                }
            },
            "status": {"status": "NeedsRemux"},
        },
    )
    write_json(root / "project-config.json", {})
    write_json(
        segment / "start-times.json",
        {"display": 0.0, "camera": 0.0, "mic": 0.3, "system_audio": 0.0},
    )
    write_json(
        segment / "cursor.json",
        {
            "moves": [
                {
                    "active_modifiers": [],
                    "cursor_id": "0",
                    "time_ms": 250.0,
                    "x": 0.5,
                    "y": 0.5,
                }
            ],
            "clicks": [
                {
                    "active_modifiers": [],
                    "cursor_id": "0",
                    "time_ms": 260.0,
                    "cursor_num": 0,
                    "down": True,
                }
            ],
        },
    )
    (segment / "keyboard.bin").write_bytes(
        b"CPKB01\x01\x01a\x04KeyA" + struct.pack("<d", 275.0) + b"\x01"
    )
    for track in ("audio-input.m4a", "system_audio.m4a"):
        (segment / track).write_bytes(b"a" * (256 * 1024 + 17))
    cursor = root / "content/cursors/arrow.png"
    cursor.parent.mkdir(parents=True)
    cursor.write_bytes(b"fixture-cursor")
    write_json(root / "shots.json", [{"shot_id": "s1", "segment": 0, "time_ms": 0.0}])
    (root / "windows.jsonl").write_text(
        json.dumps({"time_ms": 0, "segment": 0, "title": "Demo", "process": "demo.exe"})
        + "\n"
    )
    return root


def finish_take(take):
    for path in take.glob("content/segments/segment-*/*/manifest.json"):
        data = json.loads(path.read_text())
        data["is_complete"] = True
        for fragment in data["fragments"]:
            fragment["is_complete"] = True
        write_json(path, data)


@pytest.fixture
def mirror(driver, xml_server, tmp_path):
    take = create_take(tmp_path / "take.cap")
    outbox = tmp_path / "outbox"
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

    def invoke(op="scan", **fields):
        return driver(
            op,
            endpoint=xml_server.endpoint,
            config=str(config),
            take=str(take),
            outbox=str(outbox),
            **fields,
        )

    invoke.take, invoke.outbox, invoke.server, invoke.config = (
        take,
        outbox,
        xml_server,
        config,
    )
    return invoke


def shotlist():
    return {
        "shots": [
            {
                "shot_id": "intro",
                "title": "Introduce it",
                "on_screen": "The product",
                "lines": [{"sentence_id": "s1", "text": "Ship clearly."}],
            }
        ]
    }
