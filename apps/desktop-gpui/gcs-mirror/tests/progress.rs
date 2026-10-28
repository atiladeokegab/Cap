//! While a take drains, `progress.json` reports the bytes sent so far,
//! including the part of a single large PUT still on the wire.

mod common;

use common::{folder, mirror};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Like `common::bucket`, but keeps each body and holds the reply to a
/// camera PUT for `slow`, as a large upload would take.
fn slow_bucket(slow: Duration) -> (String, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let log = log.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if path.ends_with("/camera.mp4") {
                    std::thread::sleep(slow);
                }
                log.lock().unwrap().push((path, body));
                reader
                    .into_inner()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            });
        }
    });
    (endpoint, seen)
}

#[test]
fn progress_reports_the_bytes_sent_while_a_large_put_is_in_flight() {
    let (endpoint, seen) = slow_bucket(Duration::from_millis(600));
    let dir = folder("progress");
    let segment = dir.join("take/content/segments/segment-0");
    fs::write(dir.join("take/recording-meta.json"), b"{}").unwrap();
    fs::write(segment.join("display.mp4"), vec![1u8; 1000]).unwrap();
    fs::write(segment.join("camera.mp4"), vec![2u8; 5000]).unwrap();
    let mut mirror = mirror(&dir, &endpoint);
    mirror.set_progress_every(Duration::from_millis(100));
    let report = mirror.stop(1, Duration::from_secs(30));
    assert!(report.drained, "{report:?}");
    std::thread::sleep(Duration::from_millis(300)); // a heartbeat in flight may still land

    let seen = seen.lock().unwrap();
    let paths: Vec<&str> = seen.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths[0], "/b/takes/t/stop.json",
        "progress never leads the take: {paths:?}"
    );
    let progress: Vec<serde_json::Value> = seen
        .iter()
        .filter(|(path, _)| path == "/b/takes/t/progress.json")
        .map(|(_, body)| serde_json::from_slice(body).unwrap())
        .collect();
    let total = 2 + 1000 + 5000;
    assert!(
        progress.iter().all(|beat| beat["total"] == total),
        "{progress:?}"
    );
    let sent: Vec<u64> = progress
        .iter()
        .map(|beat| beat["sent"].as_u64().unwrap())
        .collect();
    // The camera's bytes count once streamed, before its PUT returns: a beat
    // logged ahead of the camera's own entry already holds them.
    let camera = seen
        .iter()
        .position(|(path, _)| path.ends_with("/camera.mp4"))
        .unwrap();
    assert!(
        seen[..camera]
            .iter()
            .any(|(path, body)| path.ends_with("/progress.json")
                && serde_json::from_slice::<serde_json::Value>(body).unwrap()["sent"]
                    .as_u64()
                    .unwrap()
                    >= 1002 + 5000),
        "{sent:?}"
    );
    assert!(sent.windows(2).all(|pair| pair[0] <= pair[1]), "{sent:?}");
    let ledger = fs::read_to_string(dir.join("outbox/uploaded.jsonl")).unwrap();
    assert!(!ledger.contains("progress.json"));
    assert!(
        !fs::read_to_string(dir.join("outbox/stop.json"))
            .unwrap()
            .contains("progress")
    );
}

#[test]
fn no_progress_is_sent_once_the_drain_has_returned() {
    let (endpoint, seen) = slow_bucket(Duration::ZERO);
    let dir = folder("progress-quiet");
    fs::write(dir.join("take/recording-meta.json"), b"{}").unwrap();
    let mut mirror = mirror(&dir, &endpoint);
    mirror.set_progress_every(Duration::from_millis(50));
    assert!(mirror.stop(1, Duration::from_secs(30)).drained);
    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(400));
    let after = seen.lock().unwrap().len();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        seen.lock().unwrap().len(),
        after,
        "still beating {:?} after the drain",
        start.elapsed()
    );
}

#[test]
fn a_put_that_outlasts_the_drain_deadline_still_lands_once() {
    let (endpoint, seen) = slow_bucket(Duration::from_millis(1500));
    let dir = folder("progress-outlasts");
    let segment = dir.join("take/content/segments/segment-0");
    fs::write(dir.join("take/recording-meta.json"), b"{}").unwrap();
    fs::write(segment.join("camera.mp4"), vec![2u8; 5000]).unwrap();
    let mut mirror = mirror(&dir, &endpoint);
    mirror.stop(1, Duration::from_millis(800));
    // the camera was not cut at the deadline; what is left is the manifest
    assert_eq!(mirror.drain(None), Ok(true));

    let seen = seen.lock().unwrap();
    let count = |key: &str| seen.iter().filter(|(path, _)| path.ends_with(key)).count();
    assert_eq!(count("/camera.mp4"), 1, "sent from its first byte again");
    // a 1 ms manifest PUT past the deadline may reach the bucket too; it is the same body
    assert!(seen.last().unwrap().0.ends_with("/take-manifest.json"));
}

#[test]
fn a_put_whose_bytes_stop_moving_fails_at_the_stall_not_the_overall_cap() {
    let (endpoint, _) = slow_bucket(Duration::from_secs(3600));
    let dir = folder("progress-stall");
    let segment = dir.join("take/content/segments/segment-0");
    fs::write(dir.join("take/recording-meta.json"), b"{}").unwrap();
    fs::write(segment.join("camera.mp4"), vec![2u8; 5000]).unwrap();
    let mut mirror = mirror(&dir, &endpoint);
    mirror.set_stall(Duration::from_millis(300));
    let start = Instant::now();
    // The camera's overall cap is 120 s; only the stall can end it this soon.
    assert!(!mirror.stop(1, Duration::from_millis(500)).drained);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}
