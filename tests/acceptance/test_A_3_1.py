import json

import pytest
from conftest import shotlist, validate_contract


def test_shotlist_loads_content_and_malformed_input_falls_back(driver, xml_server):
    good = shotlist()
    result = driver(
        "shots", shotlist=good, epoch_ms=50000, elapsed_offset_ms=9000, script=[]
    )
    validate_contract("shotlist", good)
    assert result["mode"] == "shots"
    for field in ("title", "on_screen"):
        assert result["current"][field] == good["shots"][0][field]
    assert result["current"]["lines"] == [
        line["text"] for line in good["shots"][0]["lines"]
    ]
    xml_server.objects["/acceptance-fixture/shotlist.json"] = json.dumps(good).encode()
    fetched = driver(
        "shots",
        shotlist=None,
        shotlist_url=xml_server.endpoint + "/acceptance-fixture/shotlist.json",
        access_key="acceptance-access",
        secret_key="acceptance-only-secret",
        epoch_ms=50000,
        script=[],
    )
    assert fetched["current"] == result["current"]
    assert any(method == "GET" for method, *_ in xml_server.requests)
    for invalid in ({}, {"shots": []}, {"shots": [{"shot_id": 17}]}):
        with pytest.raises(AssertionError, match="ValidationError"):
            validate_contract("shotlist", invalid)
        rejected = driver("shots", shotlist=invalid, epoch_ms=50000, script=[])
        assert rejected["mode"] == "stock" and rejected["error"]


@pytest.mark.parametrize("field", ["title", "on_screen"])
def test_optional_shotlist_fields_allow_omission_but_reject_explicit_null(
    driver, field
):
    value = shotlist()
    value["shots"][0].pop("on_screen", None)
    validate_contract("shotlist", value)
    assert driver("shots", shotlist=value, epoch_ms=0, script=[])["mode"] == "shots"
    if field == "title":
        value["title"] = None
    else:
        value["shots"][0]["on_screen"] = None
    with pytest.raises(AssertionError, match="ValidationError"):
        validate_contract("shotlist", value)
    result = driver("shots", shotlist=value, epoch_ms=0, script=[])
    assert result["mode"] == "stock" and result.get("error")
