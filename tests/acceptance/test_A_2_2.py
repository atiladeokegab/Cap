import json
import os

from conftest import file_snapshot, write_json


def test_complete_fragments_are_linked_uploaded_once_and_sources_untouched(mirror):
    before = file_snapshot(mirror.take)
    mirror()
    mirror()
    puts = mirror.server.puts()
    for track in ("display", "camera"):
        relative = f"content/segments/segment-0/{track}/fragment_000.mp4"
        matches = [(path, body) for path, body in puts if path.endswith("/" + relative)]
        assert len(matches) == 1 and matches[0][1] == before[relative]
        source, staged = mirror.take / relative, mirror.outbox / relative
        assert staged.is_file() and os.path.samefile(source, staged), (
            "Closed fragments must be hard-linked"
        )
    assert not any("fragment_001.mp4" in path for path, _ in puts)
    assert file_snapshot(mirror.take) == before


def test_wrong_manifest_size_never_publishes_the_fragment(mirror):
    path = mirror.take / "content/segments/segment-0/display/manifest.json"
    manifest = json.loads(path.read_text())
    manifest["fragments"][0]["file_size"] += 1
    write_json(path, manifest)
    mirror()
    assert not any(
        path.endswith("/display/fragment_000.mp4") for path, _ in mirror.server.puts()
    )
