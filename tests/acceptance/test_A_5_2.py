import json
import os
import shlex

from conftest import ROOT, create_take, run


def test_cap_events_reads_binary_keyboard_and_preserves_segment_times(tmp_path):
    take = create_take(tmp_path / "fixture.cap")
    cap = shlex.split(os.environ.get("CAP_CLI", "target/debug/cap"))
    assert (ROOT / cap[0]).is_file(), "Build cap CLI or set CAP_CLI to its argv"
    output = json.loads(run([*cap, "events", str(take), "--json"]))
    events = output["events"] if isinstance(output, dict) else output
    assert isinstance(events, list)
    by_kind = {
        kind: [e for e in events if e["type"] == kind]
        for kind in ("move", "click", "key")
    }
    assert len(by_kind["move"]) == len(by_kind["click"]) == len(by_kind["key"]) == 1
    for kind, expected in (("move", 250.0), ("click", 260.0), ("key", 275.0)):
        event = by_kind[kind][0]
        assert event["segment"] == 0 and event["time_ms"] == expected
    assert by_kind["key"][0]["key"] == "a"
    assert by_kind["key"][0]["key_code"] == "KeyA"
    schemas = list((ROOT / "contracts/vendor").glob("*/events*.json"))
    assert schemas, "Vendor the film-owned events schema"
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
            str(schemas[0]),
        ],
        input=json.dumps(output),
    )
