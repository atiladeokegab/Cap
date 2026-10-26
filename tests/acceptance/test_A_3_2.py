from conftest import rust_test, source


def test_next_shot_uses_injected_start_gate_clock_within_five_ms():
    source("apps/desktop-gpui/src/shots.rs")
    rust_test("acceptance_shot_clock_within_five_ms")
