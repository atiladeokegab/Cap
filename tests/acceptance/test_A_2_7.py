import os

from conftest import digest


def test_live_bucket_put_get_round_trip(driver, live, tmp_path):
    config = os.environ.get("CAP_LIVE_CONFIG")
    assert config, (
        "Set CAP_LIVE_CONFIG to the laptop-config file; never put credentials in this repo"
    )
    payload = b"cap-live-roundtrip-only"
    result = driver(
        "scan",
        config=config,
        endpoint=os.environ.get("CAP_LIVE_ENDPOINT", "https://storage.googleapis.com"),
        take=str(tmp_path),
        outbox=str(tmp_path / "outbox"),
        roundtrip_payload=payload.decode(),
    )
    assert result["roundtrip_sha256"] == digest(payload)
    assert result["roundtrip_get"] == payload.decode()
