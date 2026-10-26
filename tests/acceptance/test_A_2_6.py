import re

from conftest import ROOT, file_snapshot, run


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


def test_tracked_fork_files_do_not_contain_credentials_or_deployment_locations():
    paths = run(["git", "ls-files", "-z"]).split("\0")
    patterns = [
        r"GOOG[0-9A-Z]{15,}",
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN (?:RSA |EC )?PRIVATE KEY-----",
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        r"hub-host-ao",
        r"gs://(?!example|test|acceptance)[a-z0-9][a-z0-9._-]+",
    ]
    for relative in paths:
        if not relative or relative.startswith(("tests/", ".git/")):
            continue
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size > 2_000_000:
            continue
        text = path.read_bytes().decode("utf-8", errors="replace")
        for pattern in patterns:
            assert not re.search(pattern, text), (
                f"Possible credential/deployment location in {relative}; inspect locally"
            )
