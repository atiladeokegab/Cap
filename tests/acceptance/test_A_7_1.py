import re

from conftest import ROOT, run, source


def shortcut_limits(text):
    return re.findall(r"//+\s*shortcut:\s*([^\n]+)", text)


def test_shortcut_marker_requires_a_comment():
    text = "shortcut: &'static str,\nlet shortcut: gpui::SharedString = shortcut.into();"
    assert shortcut_limits(text) == []
    assert shortcut_limits("// shortcut: one speaker, add mixing for multiple speakers") == ["one speaker, add mixing for multiple speakers"]


def test_readme_displays_real_c4_levels_and_documents_shortcuts():
    source("docs/architecture/c4.py")
    run(["uv", "run", "--offline", "docs/architecture/c4.py"], timeout=120)
    readme = source("README.md")
    assert re.search(r"(?im)^#+\s+Known limitations", readme)
    for level in ("context", "container", "component", "code"):
        relative = f"docs/architecture/c4_{level}.png"
        image = ROOT / relative
        assert image.is_file(), f"Missing C4 level: {relative}"
        assert image.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
        assert relative in readme
    for path in (ROOT / "apps/desktop-gpui/src").rglob("*.rs"):
        for limit in shortcut_limits(path.read_text()):
            assert limit.strip() in readme, (
                f"Undocumented shortcut in {path.relative_to(ROOT)}"
            )
