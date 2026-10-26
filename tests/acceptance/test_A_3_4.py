from conftest import rust_test, source


def test_free_mode_preserves_stock_behavior_without_shot_files():
    source("apps/desktop-gpui/src/shots.rs")
    rust_test("acceptance_free_mode_writes_no_shots")
