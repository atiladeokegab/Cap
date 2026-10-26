//! Test-only adapter for tests/acceptance: one JSON request on stdin
//! (`op`: sign, scan, stop, resume, shots or windows), one JSON object on stdout.

// The teleprompter's shot logic, std and serde only, straight from the app.
#[allow(dead_code)]
#[path = "../../../src/shots.rs"]
mod shots;

// The window log's change detector and epoch arithmetic, std and serde only, straight from the app.
#[path = "../../../src/window_log/change.rs"]
mod window_change;

use cap_gcs_mirror::{Config, Mirror, sigv4};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
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

/// Window samples at `at_ms` (unix ms) through the app's change detector, timed from the
/// start-gate `epoch_ms`; returns the windows.jsonl lines the app would write.
fn windows(request: &Value) -> Result<Value, String> {
    let base = Instant::now();
    let at = |ms: &Value| base + Duration::from_millis(ms.as_u64().unwrap_or_default());
    let epoch = at(&request["epoch_ms"]);
    let segment = request["segment"].as_u64().unwrap_or_default() as u32;
    let mut detector = window_change::ChangeDetector::default();
    let mut lines = Vec::new();
    for sample in request["samples"].as_array().ok_or("samples")? {
        let window = sample["title"]
            .is_string()
            .then(|| window_change::Foreground {
                title: text(sample, "title").into(),
                process: text(sample, "process").into(),
                bounds: [0; 4],
            });
        let time_ms = window_change::time_ms(epoch, at(&sample["at_ms"]));
        lines.extend(detector.observe(segment, time_ms, window));
    }
    Ok(json!({ "lines": lines }))
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

/// Loads the shot list (inline, or a signed GET of `shotlist_url`), then
/// plays `script` on an injected clock: the start gate armed at `epoch_ms`,
/// each step at its `at_ms`. `elapsed_offset_ms` would only skew the bar's
/// timer, which marks never read.
fn shots_op(request: &Value) -> Result<Value, String> {
    let loaded = match (&request["shotlist"], request["shotlist_url"].as_str()) {
        (Value::Null, None) => None,
        (Value::Null, Some(url)) => {
            Some(fetch_shotlist(request, url).and_then(|b| shots::parse(&b)))
        }
        (list, _) => Some(shots::parse(list.to_string().as_bytes())),
    };
    let mut shots = match loaded {
        None => return Ok(json!({ "mode": "free", "marks": [] })),
        Some(Err(error)) => return Ok(json!({ "mode": "stock", "error": error, "marks": [] })),
        Some(Ok(list)) => shots::Shots::new(list),
    };
    let base = Instant::now();
    let at = |ms: f64| base + Duration::from_secs_f64(ms.max(0.) / 1000.);
    let armed = at(request["epoch_ms"].as_f64().unwrap_or(0.));
    let take = request["take"].as_str().map(PathBuf::from);
    let mut clock = shots::ShotClock::default();
    for step in request["script"].as_array().into_iter().flatten() {
        let now = at(step["at_ms"].as_f64().unwrap_or(0.));
        let action = match step["action"].as_str() {
            Some("next") => shots::Action::Next,
            Some("retake") => shots::Action::Retake,
            Some("pause") => {
                clock.pause();
                continue;
            }
            Some("resume") => {
                clock.resume(Some(now));
                continue;
            }
            other => return Err(format!("unknown action {other:?}")),
        };
        if let (Some(take), Some(time)) = (&take, clock.read(Some(armed), now)) {
            shots.mark(action, take, time).map_err(|e| e.to_string())?;
        }
    }
    // What the teleprompter shows for the shot on screen.
    let shot = shots.current();
    let lines: Vec<&str> = shot.lines.iter().map(|line| line.text.as_str()).collect();
    Ok(json!({
        "mode": "shots",
        "current": { "shot_id": shot.shot_id, "title": shot.title, "on_screen": shot.on_screen, "lines": lines },
        "marks": shots.marks(),
    }))
}

fn fetch_shotlist(request: &Value, url: &str) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let signer = sigv4::Key {
        access_key: text(request, "access_key"),
        secret_key: text(request, "secret_key"),
        region: "auto",
        service: "s3",
    };
    let client = reqwest::blocking::Client::new();
    cap_gcs_mirror::request(
        &client,
        &signer,
        "GET",
        &url,
        Vec::new(),
        Duration::from_secs(10),
    )
}

fn run(request: &Value) -> Result<Value, String> {
    if request["op"] == "sign" {
        return sign(request);
    }
    if request["op"] == "windows" {
        return windows(request);
    }
    if request["op"] == "shots" {
        return shots_op(request);
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
