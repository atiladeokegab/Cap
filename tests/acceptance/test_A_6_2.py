import json
import os
from pathlib import Path

from conftest import run


def test_actions_screenshots_have_independent_release_verdict(live, tmp_path):
    tag = os.environ.get("CAP_RELEASE_TAG")
    assert tag, (
        "Set CAP_RELEASE_TAG to the exact release whose visual review is being checked"
    )
    run(
        [
            "gh",
            "release",
            "download",
            tag,
            "-R",
            "atiladeokegab/Cap",
            "--pattern",
            "visual-verdict.json",
            "--dir",
            str(tmp_path),
        ]
    )
    verdict = json.loads((tmp_path / "visual-verdict.json").read_text())
    assert verdict["verdict"] == "pass"
    assert verdict["reviewer"] and verdict["reviewer"] != verdict["builder"]
    assert verdict["workflow_run_id"] and verdict["reference_commit"]
    for view in ("start-screen", "bottom-bar", "editor-left-rail"):
        assert verdict["views"][view]["matches_reference"] is True
        asset = verdict["views"][view]["screenshot"]
        assert Path(asset).name == asset and asset.endswith(".png")
        run(
            [
                "gh",
                "release",
                "download",
                tag,
                "-R",
                "atiladeokegab/Cap",
                "--pattern",
                asset,
                "--dir",
                str(tmp_path),
            ]
        )
        assert (tmp_path / asset).read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
