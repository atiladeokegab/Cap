import json

from conftest import digest, finish_take


def test_both_audio_tracks_use_only_new_complete_ranges_until_stop(mirror):
    mirror()
    mirror()
    for track in ("audio-input.m4a", "system_audio.m4a"):
        audio = mirror.take / "content/segments/segment-0" / track
        assert not any(path.endswith("/" + track) for path, _ in mirror.server.puts())
        first = [
            (path, body)
            for path, body in mirror.server.puts()
            if path.endswith(track + ".part-0")
        ]
        assert len(first) == 1 and first[0][1] == audio.read_bytes()[: 256 * 1024]
        with audio.open("ab") as stream:
            stream.write(b"b" * (256 * 1024))
    mirror()
    for track in ("audio-input.m4a", "system_audio.m4a"):
        parts = [
            (path, body)
            for path, body in mirror.server.puts()
            if track + ".part-" in path
        ]
        assert len(parts) == 2
        assert sorted(len(body) for _, body in parts) == [256 * 1024, 256 * 1024]
        assert any(path.endswith(".part-262144") for path, _ in parts)
    finish_take(mirror.take)
    mirror("stop", stopped_at_unix_ms=1793016000000, drain_timeout_ms=5000)
    manifests = [
        body
        for path, body in mirror.server.puts()
        if path.endswith("/take-manifest.json")
    ]
    assert len(manifests) == 1
    listed = {item["path"]: item for item in json.loads(manifests[0])["files"]}
    for track in ("audio-input.m4a", "system_audio.m4a"):
        relative = "content/segments/segment-0/" + track
        whole = [
            body for path, body in mirror.server.puts() if path.endswith("/" + relative)
        ]
        assert len(whole) == 1 and whole[0] == (mirror.take / relative).read_bytes()
        assert listed[relative]["sha256"] == digest(whole[0])
