import re

from conftest import source


def test_windows_dispatch_fixture_uses_real_clicks_and_keeps_take():
    workflow = source(".github/workflows/verify-recording.yml")
    # Strip YAML/shell line comments and PowerShell block comments, so a comment cannot satisfy a check.
    workflow = re.sub(r"(?s)<#.*?#>", "", workflow)
    workflow = re.sub(r"(?m)(?:^|\s)#.*$", "", workflow)
    assert "workflow_dispatch:" in workflow
    windows = re.search(
        r"(?ms)^  [\w-]+:\s*\n(?:(?!^  [\w-]+:).)*?runs-on:\s*['\"]?windows-2022.*?(?=^  [\w-]+:|\Z)",
        workflow,
    )
    assert windows, "Need a dedicated windows-2022 recording-fixture job"
    job = windows.group()
    assert "cap-verify-stimulus" in job
    assert "SendInput" in job, (
        "The stimulus must change in response to real scripted clicks"
    )
    assert re.search(r"studio|--mode\s+studio", job, re.IGNORECASE)
    assert re.search(r"uses:\s*actions/upload-artifact@", job)
    assert re.search(r"\.cap|keep_artifacts|recordings", job)
