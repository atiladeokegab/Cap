import os
import re

from conftest import run, source


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
    # cargo prints "N passed; 0 failed", so match pytest's summary of the selection
    assert re.search(r"\b2 passed, \d+ deselected\b", output), (
        "make check must execute acceptance tests, not just echo a recipe"
    )
