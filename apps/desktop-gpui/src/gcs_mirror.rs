//! Streams each Studio take to the bucket named in `<app data>/mirror.json`
//! while it records (design D10). Without that file nothing starts. The
//! upload logic lives in the `cap-gcs-mirror` crate; this is the app's wiring.

use cap_gcs_mirror::{Config, Mirror};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCAN_EVERY: Duration = Duration::from_secs(1);
const STOP_DRAIN: Duration = Duration::from_secs(120);

struct Running {
    take: PathBuf,
    gate: Option<cap_recording::RecordingStartGate>,
    stopping: Arc<AtomicBool>,
    stop: mpsc::Sender<(u64, mpsc::Sender<()>)>,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

fn config() -> Option<Config> {
    Config::load(&crate::store::app_data_dir().join("mirror.json"))
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
    let Some(mut mirror) = config().and_then(|config| open(config, take)) else {
        return;
    };
    let stopping = Arc::new(AtomicBool::new(false));
    let (stop, stops) = mpsc::channel::<(u64, mpsc::Sender<()>)>();
    let (scan_stopping, dir) = (stopping.clone(), take.to_path_buf());
    let spawned = std::thread::Builder::new()
        .name("gcs-mirror".into())
        .spawn(move || {
            loop {
                match stops.recv_timeout(SCAN_EVERY) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // A deleted take has nothing left to mirror.
                        if !dir.is_dir() {
                            return;
                        }
                        if let Err(error) = mirror.scan(&scan_stopping) {
                            tracing::debug!(%error, "Mirror scan incomplete; next pass retries");
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Ok((stopped_at_unix_ms, done)) => {
                        let report = mirror.stop(stopped_at_unix_ms, STOP_DRAIN);
                        tracing::info!(?report, "Mirror stop sequence finished");
                        let _ = done.send(());
                        if !report.drained {
                            mirror.drain(None);
                        }
                        return;
                    }
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "Mirror thread did not start");
        return;
    }
    let mut running = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    running.retain(|entry| entry.take != take);
    running.push(Running {
        take: take.to_path_buf(),
        gate,
        stopping,
        stop,
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

/// Runs the stop sequence for `take` and blocks until it has frozen the take
/// and drained for at most 120 s, so it must run before Cap's remux. Uploads
/// still pending carry on in the background.
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
    entry.stopping.store(true, Ordering::Relaxed);
    let (done, finished) = mpsc::channel();
    if entry.stop.send((at, done)).is_ok() {
        // The bound is the thread's; the margin covers a scan request in flight.
        let _ = finished.recv_timeout(STOP_DRAIN + Duration::from_secs(40));
    }
}

/// After a restart, finishes uploading every take whose stop ran but whose
/// manifest never landed, from its outbox.
pub fn resume_pending() {
    let Some(config) = config() else {
        return;
    };
    let pending: Vec<PathBuf> = std::fs::read_dir(crate::recording::recordings_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|take| take.join("outbox/stop.json").is_file())
        .collect();
    if pending.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("gcs-mirror-resume".into())
        .spawn(move || {
            for take in pending {
                // A take whose manifest is in its ledger returns at once.
                if let Some(mut mirror) = open(config.clone(), &take) {
                    mirror.drain(None);
                }
            }
        });
}
