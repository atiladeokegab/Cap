import json
import re

from conftest import source


def test_updater_disabled_and_telemetry_opt_in():
    config = json.loads(source("apps/desktop-gpui/tauri.conf.json"))
    assert config["bundle"]["createUpdaterArtifacts"] is False
    main = re.sub(r"//[^\n]*", "", source("apps/desktop-gpui/src/main.rs"))
    assert not re.search(r"updates\s*::\s*schedule_startup_check\s*\(", main)
    store = re.sub(r"//[^\n]*", "", source("apps/desktop-gpui/src/store.rs"))
    assert re.search(r'bool_at\s*\(\s*general\s*,\s*"enableTelemetry"\s*,\s*false\s*\)', store)
