import json
import re

from conftest import ROOT, source


def test_theme_uses_reference_palette_and_type():
    reference = ROOT / "reference/palette.json"
    assert reference.is_file(), "The independent Canvid reference palette is required"
    palette = json.loads(reference.read_text())
    theme = source("apps/desktop-gpui/src/theme.rs").lower()
    colors = palette["colors"]
    typography = palette["typography"]
    assert (
        isinstance(colors, dict)
        and colors
        and isinstance(typography, dict)
        and typography
    )
    for name, value in colors.items():
        assert re.fullmatch(r"#[0-9a-fA-F]{6}(?:[0-9a-fA-F]{2})?", value), name
        assert value[1:].lower() in theme, f"Reference color absent from theme: {name}"
    for name, family in typography.items():
        assert isinstance(family, str) and family.strip(), name
        assert family.lower() in theme, f"Reference type absent from theme: {name}"
