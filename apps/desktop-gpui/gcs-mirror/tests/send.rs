//! The film builds a stopped take only once the presenter's Send puts
//! `send.json` beside it; Discard never sends it and deletes the take.

mod common;

use cap_gcs_mirror::{discarded, undecided};
use common::{bucket, folder, manifest_put, mirror};
use std::{
    fs,
    sync::Mutex,
    time::{Duration, Instant},
};

const NEVER: Duration = Duration::from_secs(3600);
type Seen = Mutex<Vec<(String, String)>>;

fn count(seen: &Seen, method: &str, key: &str) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(m, path)| m == method && path.ends_with(&format!("/{key}")))
        .count()
}

#[test]
fn a_stopped_take_uploads_but_waits_for_send() {
    let (endpoint, seen) = bucket();
    let dir = folder("send-wait");
    fs::write(dir.join("take/shots.json"), b"[]").unwrap();
    let worker = mirror(&dir, &endpoint).spawn(NEVER).unwrap();
    worker.stop(1).unwrap();
    // The drain carries on in the background while the presenter decides.
    let until = Instant::now() + Duration::from_secs(10);
    while !manifest_put(&seen) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(manifest_put(&seen));
    let outbox = dir.join("outbox");
    let stop: serde_json::Value =
        serde_json::from_slice(&fs::read(outbox.join("stop.json")).unwrap()).unwrap();
    assert_eq!(
        stop["send_required"], true,
        "the watcher would build it unasked"
    );
    assert!(undecided(&outbox));
    assert_eq!(count(&seen, "PUT", "send.json"), 0);

    let mut sending = mirror(&dir, &endpoint);
    sending.send_to_film().unwrap();
    assert_eq!(count(&seen, "PUT", "send.json"), 1);
    assert!(!undecided(&outbox));
    let first = fs::read(outbox.join("send.json")).unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(sent["send"], true);
    // A retry, as after a restart, keeps the first decision and sends no copy.
    mirror(&dir, &endpoint).send_to_film().unwrap();
    assert_eq!(fs::read(outbox.join("send.json")).unwrap(), first);
    assert_eq!(count(&seen, "PUT", "send.json"), 1);
}

#[test]
fn discard_deletes_the_take_and_never_sends_it() {
    let (endpoint, seen) = bucket();
    let dir = folder("send-discard");
    fs::write(dir.join("take/shots.json"), b"[]").unwrap();
    let cursor = "content/segments/segment-0/cursor.json";
    fs::write(dir.join("take").join(cursor), b"[1]").unwrap();
    assert!(
        mirror(&dir, &endpoint)
            .stop(1, Duration::from_secs(10))
            .drained
    );

    let outbox = dir.join("outbox");
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(discarded(&outbox) && !undecided(&outbox));
    for key in ["stop.json", "take-manifest.json", "shots.json", cursor] {
        assert_eq!(count(&seen, "DELETE", key), 1, "{key} stayed in the bucket");
    }
    // Neither a later Send nor a restart's resume sends anything of it.
    let puts = seen.lock().unwrap().len();
    let mut again = mirror(&dir, &endpoint);
    assert!(again.send_to_film().is_err());
    assert!(again.drain(None).is_err());
    assert_eq!(seen.lock().unwrap().len(), puts, "it sent or deleted more");
    assert!(!outbox.join("send.json").exists());
}

#[test]
fn a_put_that_lands_after_discard_deletes_itself() {
    let (endpoint, seen) = bucket();
    let dir = folder("send-late-put");
    let mut late = mirror(&dir, &endpoint);
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(late.put("late.bin", b"x".to_vec(), NEVER).is_err());
    assert_eq!(count(&seen, "PUT", "late.bin"), 1);
    assert_eq!(count(&seen, "DELETE", "late.bin"), 1);
}

#[test]
fn a_take_from_before_the_choice_is_never_asked_about() {
    let dir = folder("send-legacy");
    fs::write(
        dir.join("outbox/stop.json"),
        r#"{"stopped_at_unix_ms":1,"expected_files":[]}"#,
    )
    .unwrap();
    assert!(!undecided(&dir.join("outbox")));
}
