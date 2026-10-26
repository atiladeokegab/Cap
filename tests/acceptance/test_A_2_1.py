import re

import pytest
from conftest import signature


def test_aws_published_iam_vector(driver):
    result = driver(
        "sign",
        method="GET",
        url="https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
        headers={
            "content-type": "application/x-www-form-urlencoded; charset=utf-8",
            "host": "iam.amazonaws.com",
            "x-amz-date": "20150830T123600Z",
        },
        body="",
        access_key="AKIDEXAMPLE",
        secret_key="wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        region="us-east-1",
        service="iam",
    )
    assert (
        result["signature"]
        == "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
    )


@pytest.mark.parametrize("method,body", [("GET", ""), ("PUT", "recording-bytes")])
def test_gcs_xml_authorization_uses_payload_host_and_canonical_object_path(
    driver, method, body
):
    url = "https://storage.googleapis.com/acceptance-fixture/takes/space%20and%2Bplus/clip.mp4"
    headers = {"host": "storage.googleapis.com", "x-amz-date": "20261026T120000Z"}
    result = driver(
        "sign",
        method=method,
        url=url,
        headers=headers,
        body=body,
        access_key="acceptance-access",
        secret_key="acceptance-only-secret",
        region="auto",
        service="s3",
    )
    signed = result["headers"]
    normalized = {k.lower(): v for k, v in signed.items()}
    auth = normalized["authorization"]
    match = re.search(
        r"Credential=acceptance-access/([^,]+),\s*SignedHeaders=([^,]+),\s*Signature=([0-9a-f]{64})",
        auth,
    )
    assert match
    scope, names, actual = match.groups()
    assert scope == "20261026/auto/s3/aws4_request"
    assert "host" in names.split(";") and "x-amz-date" in names.split(";")
    assert actual == signature(
        method, url, signed, body.encode(), "acceptance-only-secret", scope, names
    )
