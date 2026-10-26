import json
import os
import re
import shlex
import subprocess
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]


def run(command, *, timeout=180, env=None, input=None):
    result = subprocess.run(
        command, cwd=ROOT, env={**os.environ, **(env or {})}, input=input,
        capture_output=True, text=True, timeout=timeout,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return result.stdout


def source(path):
    target = ROOT / path
    assert target.is_file(), f"Required implementation missing: {path}"
    return target.read_text()


def rust_test(name, manifest="apps/desktop-gpui/Cargo.toml"):
    output = run([
        "cargo", "test", "--offline", "--manifest-path", manifest, name,
        "--", "--nocapture",
    ], timeout=600)
    counts = re.findall(r"test result: ok\. (\d+) passed", output)
    assert sum(map(int, counts)) > 0, f"No Rust test executed for {name}:\n{output}"


@pytest.fixture
def driver():
    command = shlex.split(os.environ.get("CAP_ACCEPTANCE_DRIVER", "target/acceptance/cap-driver"))

    def invoke(op, **fields):
        assert command and (ROOT / command[0]).is_file(), (
            "Build the test-only mirror adapter or set CAP_ACCEPTANCE_DRIVER to its argv"
        )
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
        text = "\n".join(line for line in path.read_text().splitlines() if not line.lstrip().startswith("#"))
        if re.search(r"['\"]v\*['\"]", text) and "workflow_dispatch" in text and "windows-2022" in text:
            candidates.append((path, text))
    assert len(candidates) == 1, f"Expected one fork release workflow, found {[str(p) for p, _ in candidates]}"
    return candidates[0][1]
