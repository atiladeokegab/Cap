import json
import re

import pytest
from conftest import listed, one_line_warning, republished, validate_contract


def progress(done, criteria):
    return {
        "shots": [
            {"shot_id": sid, "title": sid.title(), "done": sid in done}
            for sid in ("intro", "demo", "ask")
        ],
        "criteria": criteria,
    }


CRITERIA = [
    {"id": "technical-depth", "name": "Technical depth", "covered": True},
    {"id": "wow", "name": "Wow", "covered": False},
    # No criteria.json on the film side: listed as unverified, never covered.
    {
        "id": "presentation",
        "name": "Presentation",
        "covered": False,
        "unverified": True,
    },
]


def test_whats_left_panel_follows_the_republished_progress(
    driver, xml_server, tmp_path
):
    before = {**listed("intro", "demo", "ask"), "progress": progress((), CRITERIA)}
    pickup = {**listed("demo", "ask"), "progress": progress(("intro",), CRITERIA)}
    validate_contract("shotlist", pickup)
    _, bucket = republished(xml_server, before, pickup)
    result = driver(
        "shots",
        **bucket,
        epoch_ms=0,
        script=[{"at_ms": 0, "action": "take_start", "take": str(tmp_path)}],
    )
    panel = result["progress"]
    # M is the full list, not the pick-up list on screen.
    assert re.search(r"\b1 of 3 shots\b", panel["summary"])
    expected = [("Intro", True), ("Demo", False), ("Ask", False)] + [
        (c["name"], c["covered"]) for c in CRITERIA
    ]
    assert [row["ticked"] for row in panel["rows"]] == [t for _, t in expected]
    for row, (label, _) in zip(panel["rows"], expected):
        assert label in row["label"]
    assert result["current"]["shot_id"] == "demo"
    assert result["warning"] is None


def test_no_progress_field_looks_as_today(driver):
    plain = listed("intro", "demo")
    result = driver("shots", shotlist=plain, epoch_ms=0, script=[])
    assert result["mode"] == "shots" and result["progress"] is None
    assert result["warning"] is None
    assert result["current"]["shot_id"] == "intro"


@pytest.mark.parametrize(
    "malformed",
    [
        None,
        "1 of 3",
        [],
        {"shots": "all", "criteria": []},
        {
            "shots": [{"shot_id": "intro", "title": "Intro", "done": "yes"}],
            "criteria": [],
        },
        {"shots": [], "criteria": [{"id": "wow", "name": "Wow"}]},
        {"shots": [], "criteria": [{"id": "wow", "name": "Wow", "covered": 1}]},
    ],
    ids=[
        "null",
        "string",
        "array",
        "shots-not-a-list",
        "done-not-bool",
        "no-covered",
        "covered-not-bool",
    ],
)
def test_malformed_progress_is_ignored_with_a_warning_and_never_blocks_shots(
    driver, tmp_path, malformed
):
    result = driver(
        "shots",
        shotlist={**listed("intro", "demo"), "progress": malformed},
        take=str(tmp_path),
        epoch_ms=0,
        script=[{"at_ms": 100, "action": "next"}],
    )
    assert result["mode"] == "shots" and result["progress"] is None
    one_line_warning(result)
    assert result["current"]["shot_id"] == "intro"
    marks = json.loads((tmp_path / "shots.json").read_text())
    assert [mark["shot_id"] for mark in marks] == ["intro"]
