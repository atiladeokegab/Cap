//! The stop caller resumes Cap's remux only once the take is frozen, however
//! short the drain bound, and hears about a freeze that failed.

mod common;

use common::{bucket, folder, manifest_put, mirror};
use std::{
    fs,
    time::{Duration, Instant},
};

const NEVER: Duration = Duration::from_secs(3600);

#[test]
fn stop_returns_only_after_the_whole_take_is_frozen() {
    let (endpoint, _) = bucket();
    let dir = folder("handshake");
    let audio = "content/segments/segment-0/audio-input.m4a";
    let body = vec![7u8; 8 << 20];
    fs::write(dir.join("take").join(audio), &body).unwrap();
    let worker = mirror(&dir, &endpoint).spawn(NEVER).unwrap();
    worker.stop(1).unwrap();
    // The remux may now delete the take; the outbox must already hold it.
    assert!(dir.join("outbox/stop.json").is_file());
    assert!(fs::read(dir.join("outbox").join(audio)).unwrap() == body);
}

#[cfg(unix)]
#[test]
fn a_failed_freeze_skips_the_take_and_the_remux_still_runs() {
    use std::os::unix::fs::PermissionsExt;
    let (endpoint, seen) = bucket();
    let dir = folder("handshake-fail");
    let segment = dir.join("take/content/segments/segment-0");
    fs::write(segment.join("cursor.json"), b"[1]").unwrap();
    let keys = segment.join("keyboard.bin");
    fs::write(&keys, b"keys").unwrap();
    fs::set_permissions(&keys, fs::Permissions::from_mode(0)).unwrap();
    if fs::read(&keys).is_ok() {
        return; // Running as root: permissions cannot make a file unreadable.
    }
    let worker = mirror(&dir, &endpoint).spawn(NEVER).unwrap();
    assert!(worker.stop(1).is_err());
    // The take is skipped, visibly, and Cap's files are as Cap left them.
    let marker = fs::read_to_string(dir.join("outbox/mirror-failed.json")).unwrap();
    assert!(marker.contains("keyboard.bin"));
    assert!(!dir.join("outbox/stop.json").exists());
    assert_eq!(fs::read(segment.join("cursor.json")).unwrap(), b"[1]");
    // Cap's remux runs as normal, deleting the fragment folders.
    fs::remove_dir_all(dir.join("take/content/segments")).unwrap();
    let _ = mirror(&dir, &endpoint).drain(Some(Instant::now() + Duration::from_secs(5)));
    assert!(!manifest_put(&seen));
}
