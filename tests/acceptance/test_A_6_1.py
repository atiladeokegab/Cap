import json
import re

from conftest import ROOT, source


def test_theme_uses_independently_committed_reference_palette():
    palette = json.loads((ROOT / "tests/fixtures/canvid-palette.json").read_text())
    theme = re.sub(r"//[^\n]*", "", source("apps/desktop-gpui/src/theme.rs"))
    dark = theme.rsplit("pub fn dark()", 1)[-1].lower()
    for name, value in palette["colors"].items():
        assert re.search(rf"\b{name}\s*:\s*rgb\s*\(\s*0x{value[1:]}\s*\)", dark), (
            f"Wrong dark-theme token: {name}"
        )
    assert '"Geist"' in theme
    assert '"Segoe UI"' in theme or (
        '".SystemUIFont"' in theme and 'target_os = "windows"' in theme
    )
