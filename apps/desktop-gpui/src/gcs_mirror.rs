//! Streams each Studio take to the bucket named in `<app data>/mirror.json`
//! while it records (design D10). Without that file nothing starts. The
//! upload logic lives in the `cap-gcs-mirror` crate; this is the app's wiring.

use cap_gcs_mirror::{Config, Mirror, Worker};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCAN_EVERY: Duration = Duration::from_secs(1);
const STOP_DRAIN: Duration = Duration::from_secs(120);

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

/// Runs the stop sequence for `take`; it must run before Cap's remux. Blocks
/// until the take is frozen into the outbox, then for the drain (about
/// 120 s); uploads still pending carry on in the background. A failed freeze
/// only skips mirroring this take: Cap's own recording always finishes (D10).
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
    if let Err(error) = entry.worker.stop(at, STOP_DRAIN) {
        tracing::error!(%error, take = %take.display(), "Mirror skipped this take; see outbox/mirror-failed.json");
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
                if let Some(mut mirror) = open(config.clone(), &take)
                    && let Err(error) = mirror.drain(None)
                {
                    tracing::error!(%error, take = %take.display(), "Mirror cannot complete this take");
                }
            }
        });
}
