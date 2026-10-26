import re
import shutil

from conftest import ROOT, run, source


def shortcut_limits(text):
    return re.findall(r"//+\s*shortcut:\s*([^\n]+)", text)


def test_shortcut_marker_requires_a_comment():
    text = "shortcut: &'static str,\nlet shortcut: gpui::SharedString = shortcut.into();"
    assert shortcut_limits(text) == []
    assert shortcut_limits("// shortcut: one speaker, add mixing for multiple speakers") == ["one speaker, add mixing for multiple speakers"]


def test_readme_displays_real_c4_levels_and_documents_shortcuts(tmp_path):
    source("docs/architecture/c4.py")
    outputs = [ROOT / "README.md", *ROOT.glob("docs/architecture/c4_*.png")]
    before = {path: path.read_bytes() for path in outputs}
    for name in ("apps", "crates", ".github"):
        (tmp_path / name).symlink_to(ROOT / name, target_is_directory=True)
    renderer = tmp_path / "docs/architecture/c4.py"
    renderer.parent.mkdir(parents=True)
    shutil.copy2(ROOT / "docs/architecture/c4.py", renderer)
    shutil.copy2(ROOT / "README.md", tmp_path / "README.md")
    run(["uv", "run", "--offline", str(renderer)], timeout=120)
    assert all(path.read_bytes() == data for path, data in before.items()), (
        "Architecture verification must not rewrite the checkout"
    )
    readme = source("README.md")
    assert (tmp_path / "README.md").read_text() == readme, (
        "README fork section is stale; regenerate it explicitly outside acceptance tests"
    )
    assert re.search(r"(?im)^#+\s+Known limitations", readme)
    for level in ("context", "container", "component", "code"):
        relative = f"docs/architecture/c4_{level}.png"
        image = ROOT / relative
        assert image.is_file(), f"Missing C4 level: {relative}"
        assert image.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
        assert (tmp_path / relative).read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
        assert relative in readme
    for path in (ROOT / "apps/desktop-gpui/src").rglob("*.rs"):
        for limit in shortcut_limits(path.read_text()):
            assert limit.strip() in readme, (
                f"Undocumented shortcut in {path.relative_to(ROOT)}"
            )
