import json
import os

from conftest import run


def test_windows_fixture_run_mirrors_to_ci_prefix_manifest_last(live):
    run_id = os.environ.get("CAP_FIXTURE_RUN_ID")
    assert run_id and run_id.isdigit(), (
        "Set CAP_FIXTURE_RUN_ID to the exact Windows fixture workflow run"
    )
    report = json.loads(
        run(
            [
                "gh",
                "run",
                "view",
                run_id,
                "-R",
                "atiladeokegab/Cap",
                "--json",
                "conclusion,event,workflowName",
            ]
        )
    )
    assert report["conclusion"] == "success" and report["event"] == "workflow_dispatch"
    log = run(["gh", "run", "view", run_id, "-R", "atiladeokegab/Cap", "--log"])
    assert "ci/" in log and "take-manifest.json" in log
    puts = [line for line in log.splitlines() if "PUT " in line]
    assert puts and "take-manifest.json" in puts[-1], (
        "Retain ordered upload evidence in the fixture run"
    )
