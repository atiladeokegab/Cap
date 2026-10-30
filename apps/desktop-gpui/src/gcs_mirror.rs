//! Streams each Studio take to the bucket named in `<app data>/mirror.json`
//! while it records (design D10). Without that file nothing starts. The
//! upload logic lives in the `cap-gcs-mirror` crate; this is the app's wiring.

use cap_gcs_mirror::{Config, Mirror, Worker};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCAN_EVERY: Duration = Duration::from_secs(1);

struct Running {
    take: PathBuf,
    gate: Option<cap_recording::RecordingStartGate>,
    worker: Worker,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

/// Runs at app start (via `resume_pending`) and at each recording start, so a
/// `mirror.json` just downloaded is picked up without copying it by hand.
fn config() -> Option<Config> {
    let path = crate::store::app_data_dir().join("mirror.json");
    if let Some(downloads) = dirs::download_dir() {
        Config::adopt(&path, &downloads);
    }
    Config::load(&path)
}

fn open(config: Config, take: &Path) -> Option<Mirror> {
    let name = take.file_name()?.to_string_lossy().into_owned();
    let prefix = format!("{}/{name}", config.prefix.as_deref().unwrap_or("takes"));
    Mirror::new(config, take, &take.join("outbox"), &prefix)
        .inspect_err(|error| tracing::warn!(%error, "Mirror could not open its outbox"))
        .ok()
}

/// Starts mirroring a Studio take that has just begun recording.
pub fn start(take: &Path, gate: Option<cap_recording::RecordingStartGate>) {
    let Some(mirror) = config().and_then(|config| open(config, take)) else {
        return;
    };
    let worker = match mirror.spawn(SCAN_EVERY) {
        Ok(worker) => worker,
        Err(error) => {
            tracing::warn!(%error, "Mirror thread did not start");
            return;
        }
    };
    let mut running = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    running.retain(|entry| entry.take != take);
    running.push(Running {
        take: take.to_path_buf(),
        gate,
        worker,
    });
}

/// The stop on the start-gate clock: the arm's wall time plus the monotonic
/// time since, the clock `cursor.json` counts from.
fn stopped_at_unix_ms(gate: Option<&cap_recording::RecordingStartGate>) -> u64 {
    let now = match gate.and_then(cap_recording::RecordingStartGate::armed_at) {
        Some(armed) => armed.system_time() + armed.instant().elapsed(),
        None => SystemTime::now(),
    };
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// Freezes `take` into its outbox; it must run before Cap's remux. The rest
/// uploads in the background, but the film waits for the presenter's Send
/// ([`ask`]). A failed freeze only skips mirroring this take: Cap's own
/// recording always finishes (D10).
pub fn stop(take: &Path) {
    let entry = {
        let mut running = RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(index) = running.iter().position(|entry| entry.take == take) else {
            return;
        };
        running.remove(index)
    };
    let at = stopped_at_unix_ms(entry.gate.as_ref());
    if let Err(error) = entry.worker.stop(at) {
        tracing::error!(%error, take = %take.display(), "Mirror skipped this take; see outbox/mirror-failed.json");
    }
}

/// Every take folder whose outbox `keep` accepts.
fn takes(keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    std::fs::read_dir(crate::recording::recordings_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|take| keep(&take.join("outbox")))
        .collect()
}

static ASKING: AtomicBool = AtomicBool::new(false);

/// Asks "Send to film" or "Discard" for each stopped take not yet decided,
/// one after another, on the main window (gpui prompts are window sheets).
/// Runs after each stop and at app start, so a dialog closed or lost to a
/// crash, which decides nothing, comes back then.
pub fn ask(cx: &mut gpui::App) {
    if config().is_none() {
        return;
    }
    let mut pending = takes(cap_gcs_mirror::undecided);
    if pending.is_empty() || ASKING.swap(true, Ordering::Relaxed) {
        return;
    }
    // The take just stopped first.
    pending.sort_by_key(|take| {
        std::cmp::Reverse(
            std::fs::metadata(take.join("outbox/stop.json"))
                .and_then(|meta| meta.modified())
                .ok(),
        )
    });
    let main = cx.global::<crate::app_windows::AppWindows>().main;
    cx.spawn(async move |cx| {
        for take in pending {
            let name = take
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let receiver = cx.update(|cx| {
                crate::app_windows::show_main_window(cx);
                cx.activate(true);
                main.update(cx, |_, window, cx| {
                    window.prompt(
                        gpui::PromptLevel::Info,
                        "Send this take to the film?",
                        Some(&name),
                        &[
                            gpui::PromptButton::ok("Send to film"),
                            gpui::PromptButton::new("Discard"),
                            gpui::PromptButton::cancel("Not now"),
                        ],
                        cx,
                    )
                })
            });
            let Ok(receiver) = receiver else {
                break;
            };
            match receiver.await {
                Ok(0) => decide(take, true),
                Ok(1) => decide(take, false),
                _ => {}
            }
        }
        ASKING.store(false, Ordering::Relaxed);
    })
    .detach();
}

/// Sends `send.json`, or discards the take from the bucket, on its own thread.
fn decide(take: PathBuf, send: bool) {
    let Some(config) = config() else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("gcs-mirror-send".into())
        .spawn(move || {
            let Some(mut mirror) = open(config, &take) else {
                return;
            };
            let done = if send {
                mirror.send_to_film()
            } else {
                mirror.discard()
            };
            if let Err(error) = done {
                tracing::error!(%error, take = %take.display(), send, "Mirror could not carry out the choice");
            }
        });
}

/// After a restart, finishes uploading every stopped take not discarded, and
/// the `send.json` of each one sent.
pub fn resume_pending() {
    let Some(config) = config() else {
        return;
    };
    let pending =
        takes(|outbox| outbox.join("stop.json").is_file() && !cap_gcs_mirror::discarded(outbox));
    if pending.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("gcs-mirror-resume".into())
        .spawn(move || {
            for take in pending {
                let Some(mut mirror) = open(config.clone(), &take) else {
                    continue;
                };
                if take.join("outbox/send.json").is_file()
                    && let Err(error) = mirror.send_to_film()
                {
                    tracing::error!(%error, take = %take.display(), "Mirror could not send send.json");
                }
                // A take whose manifest is in its ledger returns at once.
                if let Err(error) = mirror.drain(None) {
                    tracing::error!(%error, take = %take.display(), "Mirror cannot complete this take");
                }
            }
        });
}
