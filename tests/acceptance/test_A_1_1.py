import re

from conftest import workflow_text


def check_release(text):
    events = re.search(r"(?ms)^['\"]?on['\"]?:\s*\n(.*?)(?=^\S|\Z)", text)
    assert events, "Release must declare its triggers explicitly"
    triggers = events.group(1)
    assert "workflow_dispatch:" in triggers
    assert re.search(r"['\"]?v\*['\"]?(?=\s|$)", triggers)
    assert not re.search(r"(?m)^  (pull_request|schedule|workflow_call):", triggers)
    assert "branches:" not in triggers, "Do not spend Windows builds on branch pushes"
    jobs = re.search(r"(?ms)^jobs:\s*\n(.*)", text)
    assert jobs
    assert len(re.findall(r"(?m)^  [\w-]+:\s*$", jobs.group(1))) == 1
    assert re.search(r"runs-on:\s*['\"]?windows-2022['\"]?\s*(?:\n|$)", jobs.group(1))
    assert re.search(r"uses:\s*\S*(?:rust-cache|cache@)", jobs.group(1), re.IGNORECASE)
    assert not re.search(
        r"signpath|crabnebula|discord|turbo|apple|macos", text, re.IGNORECASE
    )


def test_release_has_only_the_cached_windows_job():
    check_release(workflow_text())
