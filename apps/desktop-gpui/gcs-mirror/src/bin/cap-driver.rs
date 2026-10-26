//! Test-only adapter for tests/acceptance: one JSON request on stdin
//! (`op`: sign, scan, stop, resume, shots, prompter or zoom), one JSON object
//! on stdout.

// The teleprompter's shot logic, std and serde only, straight from the app.
#[allow(dead_code)]
#[path = "../../../src/shots.rs"]
mod shots;

// The editor's zoom pass, std only, straight from the app.
#[allow(dead_code)]
#[path = "../../../src/natural_zoom.rs"]
mod natural_zoom;

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

/// Loads the shot list (inline, or a signed GET of `shotlist_url` as the
/// window opens), then plays `script` on an injected clock: the start gate
/// armed at `epoch_ms` for `take`, each step at its `at_ms`.
/// `elapsed_offset_ms` would only skew the bar's timer, which marks never
/// read.
///
/// Steps: `next`, `retake`, `pause`, `resume`; `take_start` (follow `take`,
/// its gate armed at the step, and read the list again), `take_stop`,
/// `reload` (read it again), and `settle` (let every read in flight land).
/// Reads run off the script, the way the window's do: a step waits up to
/// [`READ_WAIT`] for its own read, never for an older one.
fn shots_op(request: &Value) -> Result<Value, String> {
    let url = request["shotlist_url"].as_str();
    let mut shots = None;
    let mut warning = None;
    let mut reads = shots::Reads::default();
    let (landed_tx, landed) = std::sync::mpsc::channel::<(u64, Result<shots::ShotList, String>)>();
    let start_read = |reads: &mut shots::Reads| {
        let number = reads.start();
        let (request, url, landed) = (request.clone(), url.map(String::from), landed_tx.clone());
        std::thread::spawn(move || {
            let read = url
                .ok_or_else(|| "no shotlist_url".to_string())
                .and_then(|url| fetch_shotlist(&request, &url))
                .and_then(|bytes| shots::parse(&bytes));
            landed.send((number, read)).ok();
        });
        number
    };
    // A landed read: the newest replaces the list, a failed one keeps it.
    let land = |number: u64,
                read: Result<shots::ShotList, String>,
                reads: &mut shots::Reads,
                shots: &mut Option<shots::Shots>,
                warning: &mut Option<String>| {
        if !reads.land(number) {
            return;
        }
        match read {
            Ok(list) => {
                *warning = None;
                match shots {
                    Some(shots) => shots.replace(list),
                    None => *shots = Some(shots::Shots::new(list)),
                }
            }
            Err(error) => *warning = Some(error.replace('\n', " ")),
        }
    };
    // Waits for read `number` (or every read, for `None`) to land.
    let wait = |number: Option<u64>,
                reads: &mut shots::Reads,
                shots: &mut Option<shots::Shots>,
                warning: &mut Option<String>,
                in_flight: &mut u64| {
        let deadline = Instant::now() + READ_WAIT;
        while *in_flight > 0 {
            let Ok((landed_number, read)) =
                landed.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            else {
                return;
            };
            *in_flight -= 1;
            land(landed_number, read, reads, shots, warning);
            if number == Some(landed_number) {
                return;
            }
        }
    };
    let mut in_flight = 0;
    match (&request["shotlist"], url) {
        (Value::Null, None) => return Ok(json!({ "mode": "free", "marks": [] })),
        (Value::Null, Some(_)) => {
            let number = start_read(&mut reads);
            in_flight += 1;
            wait(
                Some(number),
                &mut reads,
                &mut shots,
                &mut warning,
                &mut in_flight,
            );
        }
        (list, _) => match shots::parse(list.to_string().as_bytes()) {
            Ok(list) => shots = Some(shots::Shots::new(list)),
            Err(error) => return Ok(json!({ "mode": "stock", "error": error, "marks": [] })),
        },
    }
    let base = Instant::now();
    let at = |ms: f64| base + Duration::from_secs_f64(ms.max(0.) / 1000.);
    let mut armed = at(request["epoch_ms"].as_f64().unwrap_or(0.));
    let mut take = request["take"].as_str().map(PathBuf::from);
    let mut clock = shots::ShotClock::default();
    for step in request["script"].as_array().into_iter().flatten() {
        while let Ok((number, read)) = landed.try_recv() {
            in_flight -= 1;
            land(number, read, &mut reads, &mut shots, &mut warning);
        }
        let now = at(step["at_ms"].as_f64().unwrap_or(0.));
        let action = match step["action"].as_str() {
            Some("next") => Some(shots::Action::Next),
            Some("retake") => Some(shots::Action::Retake),
            Some("pause") => {
                clock.pause();
                continue;
            }
            Some("resume") => {
                clock.resume(Some(now));
                continue;
            }
            Some("take_start") => {
                take = step["take"].as_str().map(PathBuf::from);
                (armed, clock) = (now, shots::ShotClock::default());
                if let Some(shots) = shots.as_mut() {
                    shots.follow(take.as_deref());
                }
                None
            }
            Some("take_stop") => {
                take = None;
                if let Some(shots) = shots.as_mut() {
                    shots.follow(None);
                }
                continue;
            }
            Some("reload") => None,
            Some("settle") => {
                wait(None, &mut reads, &mut shots, &mut warning, &mut in_flight);
                continue;
            }
            other => return Err(format!("unknown action {other:?}")),
        };
        let Some(action) = action else {
            // take_start and reload: the list is read again.
            if url.is_none() {
                continue;
            }
            let number = start_read(&mut reads);
            in_flight += 1;
            wait(
                Some(number),
                &mut reads,
                &mut shots,
                &mut warning,
                &mut in_flight,
            );
            if let (Some(shots), Some(take)) = (shots.as_mut(), take.as_deref()) {
                shots.follow(Some(take));
            }
            continue;
        };
        if let (Some(shots), Some(take), Some(time)) =
            (shots.as_mut(), &take, clock.read(Some(armed), now))
        {
            shots.mark(action, take, time).map_err(|e| e.to_string())?;
        }
    }
    // The end of the script settles like `settle` before reporting.
    wait(None, &mut reads, &mut shots, &mut warning, &mut in_flight);
    let Some(shots) = shots else {
        return Ok(json!({ "mode": "stock", "error": warning, "warning": warning, "marks": [] }));
    };
    // What the teleprompter shows for the shot on screen.
    let shot = shots.current();
    let lines: Vec<&str> = shot.lines.iter().map(|line| line.text.as_str()).collect();
    let progress = shots.list.progress.as_ref().map(|progress| {
        let rows: Vec<Value> = progress
            .rows()
            .into_iter()
            .map(|(label, ticked)| json!({ "label": label, "ticked": ticked }))
            .collect();
        json!({ "summary": progress.summary(), "rows": rows })
    });
    Ok(json!({
        "mode": "shots",
        "current": { "shot_id": shot.shot_id, "title": shot.title, "on_screen": shot.on_screen, "lines": lines },
        "marks": shots.marks(),
        "warning": warning.or_else(|| shots.list.warning.clone()),
        "progress": progress,
    }))
}

/// How long a step waits for its own shot-list read before going on.
const READ_WAIT: Duration = Duration::from_secs(1);

/// The teleprompter's script size by default (`store.rs`).
const SCRIPT_TEXT_PX: f32 = 30.;

/// The teleprompter between shots, on the same injected clock as `shots`:
/// `next` and `retake` are the shot keys (Next on a started take holds on the
/// next shot's card and pauses the recording; pressed again on its card it is
/// Go), `go` starts the card's 3-2-1, `scroll` reads up to `line` (past the
/// last: the STOP cue), `tick` only lets time pass. When the 3-2-1 is over the
/// recording resumes as a new segment with that step as its epoch, and the
/// held press marks there. One view per step.
fn prompter_op(request: &Value) -> Result<Value, String> {
    let script = request["script"].as_array().cloned().unwrap_or_default();
    let mut shots = match &request["shotlist"] {
        Value::Null => {
            let view = json!({
                "shot_id": null, "line": null, "cue": null,
                "recording": true, "card": null, "countdown": null,
            });
            return Ok(json!({ "mode": "free", "marks": [], "views": vec![view; script.len()] }));
        }
        list => shots::Shots::new(shots::parse(list.to_string().as_bytes())?),
    };
    let base = Instant::now();
    let at = |ms: f64| base + Duration::from_secs_f64(ms.max(0.) / 1000.);
    let armed = at(request["epoch_ms"].as_f64().unwrap_or(0.));
    let take = request["take"].as_str().map(PathBuf::from);
    let mut clock = shots::ShotClock::default();
    let (mut line, mut recording, mut go_at) = (0, true, None::<f64>);
    let mut views = Vec::new();
    for step in &script {
        let ms = step["at_ms"].as_f64().unwrap_or(0.);
        let mut press = None;
        match step["action"].as_str() {
            Some(key @ ("next" | "retake")) => {
                let action = match key {
                    "next" => shots::Action::Next,
                    _ => shots::Action::Retake,
                };
                if shots.cue == shots::Cue::Ready(action) {
                    shots.go();
                    go_at = Some(ms);
                } else if shots.hold(action) {
                    clock.pause();
                    recording = false;
                } else {
                    press = Some(action);
                }
            }
            Some("go") => {
                if shots.go().is_some() {
                    go_at = Some(ms);
                }
            }
            Some("scroll") => {
                line = step["line"].as_u64().unwrap_or(0) as usize;
                if line >= shots.current().lines.len() {
                    shots.ended();
                }
            }
            Some("tick") => {}
            other => return Err(format!("unknown action {other:?}")),
        }
        let count = go_at.and_then(|go| shots::countdown((ms - go) as u64));
        if go_at.is_some() && count.is_none() {
            go_at = None;
            clock.resume(Some(at(ms)));
            recording = true;
            if let shots::Cue::Countdown(action) = shots.cue {
                press = Some(action);
            }
        }
        if let (Some(action), Some(take), Some(time)) =
            (press, &take, clock.read(Some(armed), at(ms)))
        {
            shots.mark(action, take, time).map_err(|e| e.to_string())?;
            line = 0;
        }
        let card = shots.upcoming().map(|shot| {
            json!({
                "title": shot.title,
                "on_screen": shot.on_screen,
                "text_px": SCRIPT_TEXT_PX * shots::CARD_SCALE,
                "go": count.is_none(),
            })
        });
        views.push(json!({
            "shot_id": shots.current().shot_id,
            "line": line,
            "cue": (shots.cue == shots::Cue::Stop).then_some("stop"),
            "recording": recording,
            "card": card,
            "countdown": count,
        }));
    }
    Ok(json!({ "mode": "shots", "marks": shots.marks(), "views": views }))
}

/// The editor's zoom pass over one recording, in ms: cursor.json's `moves`
/// and `clicks` as recorded, `keys` (key-down `{time_ms}`), `duration_ms`;
/// a `shotlist` with its `marks` (shots.json) turns its `zoom: none` shots
/// off. Ease is the editor's zoom spring's settle time.
fn zoom_op(request: &Value) -> Result<Value, String> {
    let number = |value: &Value, field: &str| value[field].as_f64().unwrap_or(f64::NAN);
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let moves: Vec<natural_zoom::Move> = list(&request["moves"])
        .iter()
        .map(|m| natural_zoom::Move {
            time_ms: number(m, "time_ms"),
            x: number(m, "x"),
            y: number(m, "y"),
        })
        .collect();
    let clicks: Vec<natural_zoom::Click> = list(&request["clicks"])
        .iter()
        .map(|c| natural_zoom::Click {
            time_ms: number(c, "time_ms"),
            down: c["down"].as_bool().unwrap_or(true),
        })
        .collect();
    let keys: Vec<f64> = list(&request["keys"])
        .iter()
        .filter(|k| k["down"].as_bool().unwrap_or(true))
        .map(|k| k.as_f64().unwrap_or_else(|| number(k, "time_ms")))
        .collect();
    let mut quiet = Vec::new();
    if !request["shotlist"].is_null() {
        let shot_list = shots::parse(request["shotlist"].to_string().as_bytes())?;
        let marks: Vec<shots::ShotMark> =
            serde_json::from_value(request["marks"].clone()).map_err(|e| e.to_string())?;
        for (_, from, to) in shots::zoomless(&shot_list, &marks) {
            quiet.push((from as f64, to.map_or(f64::INFINITY, |to| to as f64)));
        }
    }
    let duration_ms = number(request, "duration_ms");
    let segments: Vec<Value> =
        natural_zoom::natural_zoom(&moves, &clicks, &keys, duration_ms, &quiet)
            .into_iter()
            .map(|z| {
                json!({
                    "start_ms": z.start_ms, "end_ms": z.end_ms, "amount": z.amount,
                    "ease_ms": natural_zoom::EASE_MS, "x": z.x, "y": z.y,
                })
            })
            .collect();
    Ok(json!({ "segments": segments }))
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
    if request["op"] == "shots" {
        return shots_op(request);
    }
    if request["op"] == "prompter" {
        return prompter_op(request);
    }
    if request["op"] == "zoom" {
        return zoom_op(request);
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
