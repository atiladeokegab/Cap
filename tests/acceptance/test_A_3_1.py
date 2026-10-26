import json

import jsonschema
import pytest
from conftest import shotlist, validate_contract


def test_shotlist_loads_content_and_malformed_input_falls_back(driver, xml_server):
    good = shotlist()
    validate_contract("shotlist", good)
    result = driver(
        "shots", shotlist=good, epoch_ms=50000, elapsed_offset_ms=9000, script=[]
    )
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
        with pytest.raises(jsonschema.ValidationError):
            validate_contract("shotlist", invalid)
        rejected = driver("shots", shotlist=invalid, epoch_ms=50000, script=[])
        assert rejected["mode"] == "stock" and rejected["error"]
