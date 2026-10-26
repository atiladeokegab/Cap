from conftest import rust_test, source


def test_retake_appends_and_pause_resume_changes_segment():
    source("apps/desktop-gpui/src/shots.rs")
    rust_test("acceptance_shot_retake_and_new_segment")
