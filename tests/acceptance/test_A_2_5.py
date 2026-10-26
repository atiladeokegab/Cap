import json
import re

from conftest import digest, file_snapshot, finish_take, validate_contract


def test_final_manifest_is_last_complete_hashed_and_versioned(mirror):
    finish_take(mirror.take)
    expected = file_snapshot(mirror.take)
    mirror("stop", stopped_at_unix_ms=1793016000000, drain_timeout_ms=1000)
    puts = mirror.server.puts()
    assert puts[0][0].endswith("/stop.json")
    assert puts[-1][0].endswith("/take-manifest.json")
    manifest = json.loads(puts[-1][1])
    files = {item["path"]: item for item in manifest["files"]}
    assert len(files) == len(manifest["files"])
    assert set(files) == set(expected)
    for relative, body in expected.items():
        assert files[relative]["size"] == len(body)
        assert files[relative]["sha256"] == digest(body)
        assert any(
            path.endswith("/" + relative) and uploaded == body
            for path, uploaded in puts[:-1]
        )
    assert re.fullmatch(r"[a-f0-9]{40}", manifest["fork_commit"])
    assert re.fullmatch(r"\d+\.\d+\.\d+", manifest["contracts_version"])
    assert manifest["project_format_version"] is not None
    validate_contract("take-manifest", manifest, manifest["contracts_version"])
    for relative, body in expected.items():
        assert (mirror.take / relative).read_bytes() == body
