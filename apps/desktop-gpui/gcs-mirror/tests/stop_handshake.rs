//! The stop caller resumes Cap's remux only once the take is frozen, however
//! short the drain bound, and hears about a freeze that failed.

mod common;

use common::{bucket, folder, mirror};
use std::{fs, time::Duration};

const NEVER: Duration = Duration::from_secs(3600);

#[test]
fn stop_returns_only_after_the_whole_take_is_frozen() {
    let (endpoint, _) = bucket();
    let dir = folder("handshake");
    let audio = "content/segments/segment-0/audio-input.m4a";
    let body = vec![7u8; 8 << 20];
    fs::write(dir.join("take").join(audio), &body).unwrap();
    let worker = mirror(&dir, &endpoint).spawn(NEVER).unwrap();
    worker.stop(1, Duration::ZERO).unwrap();
    // The remux may now delete the take; the outbox must already hold it.
    assert!(dir.join("outbox/stop.json").is_file());
    assert!(fs::read(dir.join("outbox").join(audio)).unwrap() == body);
}

#[cfg(unix)]
#[test]
fn a_failed_freeze_reaches_the_caller() {
    use std::os::unix::fs::PermissionsExt;
    let (endpoint, _) = bucket();
    let dir = folder("handshake-fail");
    let keys = dir.join("take/content/segments/segment-0/keyboard.bin");
    fs::write(&keys, b"keys").unwrap();
    fs::set_permissions(&keys, fs::Permissions::from_mode(0)).unwrap();
    if fs::read(&keys).is_ok() {
        return; // Running as root: permissions cannot make a file unreadable.
    }
    let worker = mirror(&dir, &endpoint).spawn(NEVER).unwrap();
    assert!(worker.stop(1, Duration::ZERO).is_err());
}
