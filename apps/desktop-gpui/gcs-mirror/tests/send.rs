//! The film builds a stopped take only once the presenter's Send puts
//! `send.json` beside it; Discard never sends it and deletes the take.

mod common;

use cap_gcs_mirror::{discard_pending, discarded, undecided};
use common::{bucket, folder, manifest_put, mirror};
use percent_encoding::percent_decode_str;
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
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

/// A bucket that keeps object names, answers LIST, and can go offline, keep
/// a PUT but lose its response, hold PUTs until released, or fail a DELETE.
/// Each connection runs on its own thread, so a held PUT blocks nothing else.
#[derive(Default)]
struct Store {
    objects: BTreeSet<String>,
    offline: bool,
    lose_next_put: bool,
    hold_puts: bool,
    puts_waiting: usize,
    fail_next_delete: bool,
    /// A LIST takes its names on arrival but answers only once released.
    hold_lists: bool,
    lists_waiting: usize,
}

fn store() -> (String, Arc<Mutex<Store>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(Store::default()));
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let target = parts.next().unwrap_or_default().to_string();
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
                reader.read_exact(&mut vec![0; length]).unwrap();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let key = percent_decode_str(path.trim_start_matches("/b/"))
                    .decode_utf8_lossy()
                    .into_owned();
                let mut state = shared.lock().unwrap();
                if method == "PUT" && state.hold_puts {
                    state.puts_waiting += 1;
                    while state.hold_puts {
                        drop(state);
                        std::thread::sleep(Duration::from_millis(10));
                        state = shared.lock().unwrap();
                    }
                    state.puts_waiting -= 1;
                }
                let (status, body) = if state.offline {
                    ("503 Service Unavailable", String::new())
                } else if method == "GET" && query.contains("list-type=2") {
                    let prefix = query.split("prefix=").nth(1).unwrap_or_default();
                    let prefix = percent_decode_str(prefix).decode_utf8_lossy().into_owned();
                    let keys: String = state
                        .objects
                        .iter()
                        .filter(|key| key.starts_with(&prefix))
                        .map(|key| {
                            format!(
                                "<Contents><Key>{}</Key></Contents>",
                                key.replace('&', "&amp;")
                            )
                        })
                        .collect();
                    (
                        "200 OK",
                        format!("<ListBucketResult>{keys}</ListBucketResult>"),
                    )
                } else if method == "PUT" {
                    state.objects.insert(key);
                    if std::mem::take(&mut state.lose_next_put) {
                        return; // stored, but the connection drops before any response
                    }
                    ("200 OK", String::new())
                } else if method == "DELETE" && std::mem::take(&mut state.fail_next_delete) {
                    ("503 Service Unavailable", String::new())
                } else if method == "DELETE" {
                    state.objects.remove(&key);
                    ("204 No Content", String::new())
                } else {
                    ("200 OK", String::new())
                };
                if method == "GET" && state.hold_lists {
                    state.lists_waiting += 1;
                    while state.hold_lists {
                        drop(state);
                        std::thread::sleep(Duration::from_millis(10));
                        state = shared.lock().unwrap();
                    }
                    state.lists_waiting -= 1;
                }
                drop(state);
                let _ = reader.into_inner().write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            });
        }
    });
    (endpoint, state)
}

fn objects(state: &Mutex<Store>) -> Vec<String> {
    state.lock().unwrap().objects.iter().cloned().collect()
}

const NEIGHBOURS: [&str; 2] = ["takes/t2/camera.mp4", "takes/other/camera.mp4"];

fn with_neighbours(state: &Mutex<Store>) {
    state
        .lock()
        .unwrap()
        .objects
        .extend(NEIGHBOURS.map(String::from));
}

#[test]
fn discard_deletes_every_object_of_the_take_even_one_whose_response_was_lost() {
    let (endpoint, state) = store();
    with_neighbours(&state);
    let dir = folder("send-discard");
    let mut streaming = mirror(&dir, &endpoint);
    streaming.put("shots.json", b"[]".to_vec(), NEVER).unwrap();
    // The bucket keeps this one, but the ledger never hears of it.
    state.lock().unwrap().lose_next_put = true;
    assert!(streaming.put("camera.mp4", b"x".to_vec(), NEVER).is_err());
    assert!(objects(&state).contains(&"takes/t/camera.mp4".to_string()));

    let outbox = dir.join("outbox");
    mirror(&dir, &endpoint).discard().unwrap();
    assert_eq!(
        objects(&state),
        NEIGHBOURS
            .map(String::from)
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(discarded(&outbox) && !discard_pending(&outbox) && !undecided(&outbox));
    // Neither a later Send nor a restart's resume sends anything of it.
    let mut again = mirror(&dir, &endpoint);
    assert!(again.send_to_film().is_err());
    assert!(again.drain(None).is_err());
    assert!(!outbox.join("send.json").exists());
    assert_eq!(objects(&state).len(), 2);
}

#[test]
fn a_discard_made_offline_finishes_once_back_online() {
    let (endpoint, state) = store();
    with_neighbours(&state);
    let dir = folder("send-offline-discard");
    mirror(&dir, &endpoint)
        .put("shots.json", b"[]".to_vec(), NEVER)
        .unwrap();
    let outbox = dir.join("outbox");
    state.lock().unwrap().offline = true;
    assert!(mirror(&dir, &endpoint).discard().is_err());
    assert!(discarded(&outbox) && discard_pending(&outbox));
    assert_eq!(objects(&state).len(), 3);
    // As after a restart: the pending Discard runs again and completes.
    state.lock().unwrap().offline = false;
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(!discard_pending(&outbox));
    assert_eq!(objects(&state).len(), 2, "only the neighbours stay");
}

#[test]
fn discard_waits_for_a_put_still_running_whose_cleanup_delete_fails() {
    let (endpoint, state) = store();
    let dir = folder("send-held-put");
    let outbox = dir.join("outbox");
    state.lock().unwrap().hold_puts = true;
    let mut streaming = mirror(&dir, &endpoint);
    let upload = std::thread::spawn(move || streaming.put("camera.mp4", b"x".to_vec(), NEVER));
    while state.lock().unwrap().puts_waiting == 0 {
        std::thread::sleep(Duration::from_millis(10));
    }
    // The LIST is empty, but the PUT has not landed: Discard is not done.
    assert!(mirror(&dir, &endpoint).discard().is_err());
    assert!(discard_pending(&outbox));
    // It lands, and its own cleanup DELETE fails.
    {
        let mut state = state.lock().unwrap();
        state.fail_next_delete = true;
        state.hold_puts = false;
    }
    assert!(upload.join().unwrap().is_err());
    assert_eq!(objects(&state), ["takes/t/camera.mp4"]);
    assert!(discard_pending(&outbox));
    // The retry (in-app, or at the next start) finds it and finishes.
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(objects(&state).is_empty() && !discard_pending(&outbox));
}

#[test]
fn a_list_answered_after_a_put_lands_never_marks_the_discard_done() {
    let (endpoint, state) = store();
    let dir = folder("send-stale-list");
    let outbox = dir.join("outbox");
    {
        let mut state = state.lock().unwrap();
        state.hold_puts = true;
        state.hold_lists = true;
    }
    let mut streaming = mirror(&dir, &endpoint);
    let upload = std::thread::spawn(move || streaming.put("camera.mp4", b"x".to_vec(), NEVER));
    while state.lock().unwrap().puts_waiting == 0 {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Discard's LIST is taken (empty) while the PUT is pending, but not yet answered.
    let mut discarding = mirror(&dir, &endpoint);
    let discard = std::thread::spawn(move || discarding.discard());
    while state.lock().unwrap().lists_waiting == 0 && !discard.is_finished() {
        std::thread::sleep(Duration::from_millis(10));
    }
    // The PUT lands, its cleanup DELETE fails, and nothing is in flight any more.
    {
        let mut state = state.lock().unwrap();
        state.fail_next_delete = true;
        state.hold_puts = false;
    }
    assert!(upload.join().unwrap().is_err());
    state.lock().unwrap().hold_lists = false;
    // The stale, empty LIST must not finish the Discard.
    assert!(discard.join().unwrap().is_err());
    assert!(discard_pending(&outbox));
    assert_eq!(objects(&state), ["takes/t/camera.mp4"]);
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(objects(&state).is_empty() && !discard_pending(&outbox));
}

#[test]
fn discard_never_deletes_a_listed_name_outside_the_take() {
    let (endpoint, state) = store();
    with_neighbours(&state);
    state
        .lock()
        .unwrap()
        .objects
        .insert("takes/t/../other/camera.mp4".into());
    let dir = folder("send-climb");
    assert!(mirror(&dir, &endpoint).discard().is_err());
    assert!(discard_pending(&dir.join("outbox")));
    assert_eq!(objects(&state).len(), 3, "nothing was deleted");
}

#[test]
fn a_send_that_failed_offline_lands_once_back_online_unchanged() {
    let (endpoint, state) = store();
    let dir = folder("send-offline-send");
    let outbox = dir.join("outbox");
    state.lock().unwrap().offline = true;
    assert!(mirror(&dir, &endpoint).send_to_film().is_err());
    let decided = fs::read(outbox.join("send.json")).unwrap();
    state.lock().unwrap().offline = false;
    mirror(&dir, &endpoint).send_to_film().unwrap();
    assert_eq!(objects(&state), ["takes/t/send.json"]);
    assert_eq!(fs::read(outbox.join("send.json")).unwrap(), decided);
}

#[test]
fn a_put_started_after_discard_never_goes_up() {
    let (endpoint, seen) = bucket();
    let dir = folder("send-late-put");
    let mut late = mirror(&dir, &endpoint);
    mirror(&dir, &endpoint).discard().unwrap();
    assert!(late.put("late.bin", b"x".to_vec(), NEVER).is_err());
    assert_eq!(count(&seen, "PUT", "late.bin"), 0);
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
