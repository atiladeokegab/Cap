//! Test-only adapter for tests/acceptance: one JSON request on stdin
//! (`op`: sign, scan, stop or resume), one JSON object on stdout.

use cap_gcs_mirror::{Config, Mirror, sigv4};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn text<'a>(request: &'a Value, field: &str) -> &'a str {
    request[field].as_str().unwrap_or_default()
}

fn millis(request: &Value, field: &str, default: u64) -> Duration {
    Duration::from_millis(request[field].as_u64().unwrap_or(default))
}

fn sign(request: &Value) -> Result<Value, String> {
    let url = reqwest::Url::parse(text(request, "url")).map_err(|e| e.to_string())?;
    let mut headers: BTreeMap<String, String> =
        serde_json::from_value(request["headers"].clone()).map_err(|e| e.to_string())?;
    let key = sigv4::Key {
        access_key: text(request, "access_key"),
        secret_key: text(request, "secret_key"),
        region: text(request, "region"),
        service: text(request, "service"),
    };
    let payload = sigv4::sha256_hex(text(request, "body").as_bytes());
    let (authorization, signature) =
        sigv4::authorize(&key, text(request, "method"), &url, &headers, &payload)?;
    headers.insert("authorization".into(), authorization);
    Ok(json!({ "signature": signature, "headers": headers }))
}

fn mirror(request: &Value) -> Result<Option<Mirror>, String> {
    let Some(config) = request["config"]
        .as_str()
        .and_then(|path| Config::load(Path::new(path)))
    else {
        return Ok(None);
    };
    let prefix = config.prefix.clone().unwrap_or_else(|| "takes".into());
    let mut mirror = Mirror::new(
        config,
        Path::new(text(request, "take")),
        Path::new(text(request, "outbox")),
        &prefix,
    )
    .map_err(|e| e.to_string())?;
    if let Some(endpoint) = request["endpoint"].as_str() {
        mirror.set_endpoint(endpoint);
    }
    Ok(Some(mirror))
}

fn run(request: &Value) -> Result<Value, String> {
    if request["op"] == "sign" {
        return sign(request);
    }
    let Some(mut mirror) = mirror(request)? else {
        return Ok(json!({ "enabled": false }));
    };
    match text(request, "op") {
        "scan" => {
            mirror.scan(&AtomicBool::new(false))?;
            let Some(payload) = request["roundtrip_payload"].as_str() else {
                return Ok(json!({ "enabled": true }));
            };
            let key = "acceptance-roundtrip.txt";
            mirror.put(key, payload.as_bytes().to_vec(), Duration::from_secs(30))?;
            let body = mirror.get(key)?;
            Ok(json!({
                "enabled": true,
                "roundtrip_sha256": sigv4::sha256_hex(&body),
                "roundtrip_get": String::from_utf8_lossy(&body),
            }))
        }
        "stop" => {
            let report = mirror.stop(
                request["stopped_at_unix_ms"]
                    .as_u64()
                    .ok_or("stopped_at_unix_ms")?,
                millis(request, "drain_timeout_ms", 120_000),
            );
            Ok(json!({
                "enabled": true,
                "frozen": report.frozen,
                "drained": report.drained,
                "error": report.error,
            }))
        }
        "resume" => {
            let deadline = Instant::now() + millis(request, "drain_timeout_ms", 15_000);
            Ok(match mirror.drain(Some(deadline)) {
                Ok(drained) => json!({ "enabled": true, "drained": drained }),
                Err(error) => json!({ "enabled": true, "drained": false, "error": error }),
            })
        }
        op => Err(format!("unknown op {op:?}")),
    }
}

fn main() {
    let request: Value =
        serde_json::from_reader(std::io::stdin()).expect("one JSON request on stdin");
    match run(&request) {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
