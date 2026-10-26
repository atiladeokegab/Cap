//! A manifest only ever certifies a take whose every expected file was frozen,
//! matches `stop.json` and went up. Each case once published a manifest.

mod common;

use common::{bucket, folder, manifest_put, mirror};
use std::{
    fs,
    time::{Duration, Instant},
};

fn stop_json(path: &str, body: &[u8]) -> String {
    format!(
        r#"{{"stopped_at_unix_ms":1,"expected_files":[{{"path":{path:?},"size":{},"sha256":"{}"}}]}}"#,
        body.len(),
        cap_gcs_mirror::sigv4::sha256_hex(body)
    )
}

#[test]
fn a_file_missing_from_the_outbox_never_gets_a_manifest() {
    let (endpoint, seen) = bucket();
    let dir = folder("missing");
    fs::write(
        dir.join("outbox/stop.json"),
        stop_json("content/a.m4a", b"audio"),
    )
    .unwrap();
    let _ = mirror(&dir, &endpoint).drain(Some(Instant::now() + Duration::from_secs(5)));
    assert!(!manifest_put(&seen));
    assert!(!dir.join("outbox/take-manifest.json").exists());
}

#[test]
fn a_file_changed_after_hashing_never_gets_a_manifest() {
    let (endpoint, seen) = bucket();
    let dir = folder("corrupt");
    let cursor = "content/segments/segment-0/cursor.json";
    fs::write(dir.join("take").join(cursor), b"[1,2,3]").unwrap();
    let report = mirror(&dir, &endpoint).stop(1, Duration::ZERO);
    assert!(report.frozen && !report.drained);
    fs::write(dir.join("outbox").join(cursor), b"[9,9,9]").unwrap();
    let _ = mirror(&dir, &endpoint).drain(Some(Instant::now() + Duration::from_secs(5)));
    assert!(!manifest_put(&seen));
    let puts = seen.lock().unwrap();
    assert!(!puts.iter().any(|(_, path)| path.ends_with(cursor)));
}

#[test]
fn a_path_outside_the_take_is_never_read_or_certified() {
    let (endpoint, seen) = bucket();
    let dir = folder("outside");
    fs::write(dir.join("private-proof"), b"private").unwrap();
    fs::write(
        dir.join("outbox/stop.json"),
        stop_json("../private-proof", b"private"),
    )
    .unwrap();
    let _ = mirror(&dir, &endpoint).drain(Some(Instant::now() + Duration::from_secs(5)));
    assert!(!manifest_put(&seen));
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, path)| path.contains("private-proof"))
    );
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_hashed_fails_the_stop_instead_of_shrinking_it() {
    use std::os::unix::fs::PermissionsExt;
    let (endpoint, seen) = bucket();
    let dir = folder("unhashable");
    let take = dir.join("take/content/segments/segment-0");
    fs::write(take.join("cursor.json"), b"[]").unwrap();
    fs::write(take.join("keyboard.bin"), b"keys").unwrap();
    fs::set_permissions(take.join("keyboard.bin"), fs::Permissions::from_mode(0)).unwrap();
    if fs::read(take.join("keyboard.bin")).is_ok() {
        return; // Running as root: permissions cannot make a file unreadable.
    }
    let report = mirror(&dir, &endpoint).stop(1, Duration::from_secs(5));
    assert!(!report.frozen && !report.drained);
    assert!(!manifest_put(&seen));
    if let Ok(stop) = fs::read_to_string(dir.join("outbox/stop.json")) {
        assert!(
            stop.contains("keyboard.bin"),
            "stop.json dropped the unreadable file"
        );
    }
}
