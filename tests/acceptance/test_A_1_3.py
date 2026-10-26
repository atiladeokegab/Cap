import os
import re

from conftest import run, source


def acceptance_ran(output):
    return re.search(r"\b2 passed, \d+ deselected\b", output)


def test_summary_requires_pytest_acceptance_not_cargo_passes():
    assert acceptance_ran("2 passed, 35 deselected in 0.12s")
    assert not acceptance_ran("test result: ok. 1550 passed; 0 failed")


def test_make_check_runs_the_offline_spine_without_recursion():
    assert not os.environ.get("CAP_ACCEPTANCE_NESTED"), (
        "make check ignored the spine test selection"
    )
    source("Makefile")
    source("docs/stack.md")
    output = run(
        ["make", "check"],
        timeout=900,
        env={
            "CAP_ACCEPTANCE_NESTED": "1",
            "PYTEST_ADDOPTS": "-k 'test_A_1_1 or test_A_1_2'",
            "CARGO_NET_OFFLINE": "true",
            "UV_OFFLINE": "1",
            "GIT_TERMINAL_PROMPT": "0",
        },
    )
    assert acceptance_ran(output), (
        "make check must execute acceptance tests, not just echo a recipe"
    )
