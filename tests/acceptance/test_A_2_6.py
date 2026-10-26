import os
import re
from pathlib import Path

import pytest
from conftest import ROOT, file_snapshot, run

PUBLIC_FIXTURES = {
    "apps/media-server/src/__tests__/routes/peer-jobs.test.ts": ["10.0.0.1"],
    "apps/mobile/scripts/mobile-development-network.test.mjs": [
        "10.8.0.2",
        "172.20.10.3",
        "192.168.1.12",
        "192.168.4.108",
    ],
    "apps/web/__tests__/unit/desktop-session-port.test.ts": ["192.168.0.25"],
    "apps/web/__tests__/unit/developer-domain-validation.test.ts": ["192.168.1.1"],
    "apps/web/__tests__/unit/mobile-request-origin.test.ts": [
        "10.0.0.4",
        "192.168.4.42",
    ],
    "apps/web/__tests__/unit/organization-storage-s3-endpoint.test.ts": [
        "10.0.0.5",
        "192.168.1.10",
    ],
    "apps/web/__tests__/unit/rate-limit.test.ts": ["10.0.0.1"],
    "apps/web/__tests__/unit/s3-bucket-access.test.ts": ["10.0.0.42"],
    "apps/web/__tests__/unit/s3-endpoint-policy.test.ts": [
        "10.0.0.1",
        "10.0.0.5",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.10",
        "metadata.google.internal",
    ],
    "apps/web/lib/s3-endpoint-policy.ts": ["10.0.0.0", "172.16.0.0", "192.168.0.0"],
    "apps/web/components/pages/HomeTwo/showcase/ownership.tsx": [
        "recordings.acme.internal"
    ],
}


def test_absent_config_never_starts_the_mirror(driver, xml_server, tmp_path):
    before = file_snapshot(tmp_path)
    result = driver(
        "scan",
        endpoint=xml_server.endpoint,
        config=None,
        take=str(tmp_path / "missing.cap"),
        outbox=str(tmp_path / "outbox"),
    )
    assert result["enabled"] is False
    assert not xml_server.requests
    assert file_snapshot(tmp_path) == before


def tracked_texts():
    for relative in run(["git", "ls-files", "-z"]).split("\0"):
        if not relative:
            continue
        path = ROOT / relative
        if path.is_file() and path.stat().st_size <= 2_000_000:
            yield relative, path.read_bytes().decode("utf-8", errors="replace")


def test_tracked_fork_files_do_not_contain_credentials_or_deployment_locations():
    patterns = [
        r"GOOG[0-9A-Z]{15,}",
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN (?:RSA |EC )?PRIVATE KEY-----",
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        r"\b(?:10\.[0-9]{1,3}|192\.168|172\.(?:1[6-9]|2[0-9]|3[01]))\.[0-9]{1,3}\.[0-9]{1,3}\b",
        r"(?:https?://|[\"'])(?:[A-Za-z0-9-]+\.)+internal\b",
        r"gs://(?!example|test|acceptance)[a-z0-9][a-z0-9._-]+",
        r"storage\.googleapis\.com/(?!acceptance-fixture|example|test)[a-z0-9][a-z0-9._-]+",
    ]
    for relative, text in tracked_texts():
        text = text.replace("host.docker.internal", "local-development-host")
        allowed = PUBLIC_FIXTURES.get(relative, [])
        if relative == "tests/acceptance/test_A_2_6.py":
            allowed = [
                literal for literals in PUBLIC_FIXTURES.values() for literal in literals
            ]
        for literal in sorted(allowed, key=len, reverse=True):
            text = text.replace(literal, "public-fixture")
        for pattern in patterns:
            if re.search(pattern, text):
                pytest.fail(
                    f"Possible credential/deployment location in {relative}; inspect locally",
                    pytrace=False,
                )


def test_private_deployment_denylist_is_not_published():
    denylist = os.environ.get("CAP_DENYLIST")
    if not denylist:
        pytest.skip(
            "Private deployment-literal scan needs CAP_DENYLIST pointing outside the public repo"
        )
    path = Path(denylist).resolve()
    assert not path.is_relative_to(ROOT)
    literals = [
        line.strip()
        for line in path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    assert literals
    for relative, text in tracked_texts():
        if any(literal in text for literal in literals):
            pytest.fail(
                f"Private deployment literal in {relative}; inspect locally",
                pytrace=False,
            )
