import json
import os
import shutil

from conftest import run


def test_tag_release_contains_an_unsigned_nsis_installer(live, tmp_path):
    tag = os.environ.get("CAP_RELEASE_TAG")
    assert tag and tag.startswith("v"), (
        "Set CAP_RELEASE_TAG to the exact release being checked"
    )
    release = json.loads(
        run(
            [
                "gh",
                "release",
                "view",
                tag,
                "-R",
                "atiladeokegab/Cap",
                "--json",
                "tagName,isDraft,assets",
            ]
        )
    )
    assert release["tagName"] == tag and release["isDraft"] is False
    assets = [a["name"] for a in release["assets"] if a["name"].endswith(".exe")]
    assert assets, "No Windows installer attached to the tag"
    assert shutil.which("osslsigncode"), (
        "Install osslsigncode to verify the unsigned release"
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
            assets[0],
            "--dir",
            str(tmp_path),
        ]
    )
    import subprocess

    result = subprocess.run(
        ["osslsigncode", "verify", "-in", str(tmp_path / assets[0])],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert "No signature found" in result.stdout + result.stderr
