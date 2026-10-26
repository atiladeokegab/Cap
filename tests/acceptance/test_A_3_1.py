from conftest import ROOT, rust_test, source


def test_shotlist_validation_loading_and_invalid_input_fallback():
    schemas = list((ROOT / "contracts/vendor").glob("*/shotlist*.json"))
    assert schemas, (
        "Vendor the film-owned shotlist schema before accepting untrusted shot lists"
    )
    source("apps/desktop-gpui/src/shots.rs")
    rust_test("acceptance_shotlist_validation_and_fallback")
