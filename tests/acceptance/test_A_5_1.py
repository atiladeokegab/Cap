from conftest import rust_test, source


def test_window_detector_uses_epoch_and_emits_only_changes_on_linux():
    source("apps/desktop-gpui/src/window_log.rs")
    rust_test("acceptance_window_log_change_detector")
