import json
import re

from conftest import ROOT, source


def test_light_theme_uses_the_measured_reference_palette():
    # reference/palette.json is measured from the lead's Canvid light-app screenshots
    palette = json.loads((ROOT / "reference/palette.json").read_text())
    hexes = set(re.findall(r"#([0-9a-f]{6})", json.dumps(palette["colors"]).lower()))
    assert len(hexes) >= 20, "Palette lost colours"
    theme = re.sub(r"//[^\n]*", "", source("apps/desktop-gpui/src/theme.rs")).lower()
    light = "".join(
        re.split(r"\bfn\s", body, maxsplit=1)[0]
        for body in theme.split("pub fn light()")[1:]
    )
    assert light, "theme.rs needs a light theme"
    tokens = set(re.findall(r"\brgb\s*\(\s*0x([0-9a-f]{6})\s*\)", light))
    missing = sorted(hexes - tokens)
    assert not missing, f"Light theme lacks measured colours: {missing}"
    assert '"inter"' in theme, "Inter is the measured stand-in face"
