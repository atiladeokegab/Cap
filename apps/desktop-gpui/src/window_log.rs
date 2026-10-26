//! The window log (design D18): while a take records, poll the foreground window every 250 ms and
//! append one line to the take's `windows.jsonl` each time it changes. Times are on the take's
//! start-gate clock, the same epoch `cursor.json` uses, so a title lines up with the clicks under it.
//! Only Windows has a poller; other platforms compile it out and log nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod change;

use change::{ChangeDetector, Foreground, WindowLine};

pub const WINDOWS_LOG_FILE_NAME: &str = "windows.jsonl";
const POLL: Duration = Duration::from_millis(250);

/// Stops the poller when dropped.
pub struct WindowLog {
    stop: Arc<AtomicBool>,
}

impl Drop for WindowLog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts logging one segment into `<take>/windows.jsonl`. `epoch` returns the segment's input epoch
/// (the start gate's armed instant, as cursor.json uses) once the gate opens; nothing is written before.
pub fn spawn(
    take: &Path,
    segment: u32,
    epoch: impl Fn() -> Option<Instant> + Send + 'static,
) -> WindowLog {
    let stop = Arc::new(AtomicBool::new(false));
    let path = take.join(WINDOWS_LOG_FILE_NAME);
    let flag = stop.clone();
    if cfg!(windows) {
        std::thread::Builder::new()
            .name("window-log".into())
            .spawn(move || poll(path, segment, epoch, flag))
            .map_err(|e| tracing::warn!("window log not started: {e}"))
            .ok();
    }
    WindowLog { stop }
}

/// The studio take being logged: its folder, its current segment and, unless paused, that segment's log.
struct Current {
    take: PathBuf,
    segment: u32,
    log: Option<WindowLog>,
}

static CURRENT: Mutex<Option<Current>> = Mutex::new(None);

/// Segment 0 of a studio take; `epoch` is as for [`spawn`].
pub fn start(take: &Path, epoch: impl Fn() -> Option<Instant> + Send + 'static) {
    let log = spawn(take, 0, epoch);
    *lock() = Some(Current {
        take: take.to_path_buf(),
        segment: 0,
        log: Some(log),
    });
}

/// Pause ends the segment's log; resume starts the next segment, timed from now.
// shortcut: the resume is seen when the actor acknowledges it, a little after the new segment's
// epoch, so its first titles read slightly late.
pub fn set_paused(paused: bool) {
    let mut current = lock();
    let Some(current) = current.as_mut() else {
        return;
    };
    if paused {
        current.log = None;
    } else if current.log.is_none() {
        current.segment += 1;
        let at = Instant::now();
        current.log = Some(spawn(&current.take, current.segment, move || Some(at)));
    }
}

/// Stop, cancel or delete of `take`: no more lines for it. A take started since (a restart, or a
/// stale start discarded late) keeps its log.
pub fn stop(take: &Path) {
    let mut current = lock();
    if current.as_ref().is_some_and(|current| current.take == take) {
        *current = None;
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<Current>> {
    CURRENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn poll(path: PathBuf, segment: u32, epoch: impl Fn() -> Option<Instant>, stop: Arc<AtomicBool>) {
    let mut detector = ChangeDetector::default();
    while !stop.load(Ordering::Relaxed) {
        if let Some(epoch) = epoch() {
            let time_ms = change::time_ms(epoch, Instant::now());
            let line = detector.observe(segment, time_ms, foreground());
            // A segment that ended mid-poll writes nothing more, so segments never interleave.
            if let Some(line) = line.filter(|_| !stop.load(Ordering::Relaxed))
                && let Err(e) = append(&path, &line)
            {
                tracing::warn!("window log write failed: {e}");
            }
        }
        std::thread::sleep(POLL);
    }
}

fn append(path: &Path, line: &WindowLine) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(line)?)
}

#[cfg(windows)]
fn foreground() -> Option<Foreground> {
    use windows_sys::Win32::Foundation::{CloseHandle, RECT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut title = [0u16; 512];
        let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut process = String::new();
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !handle.is_null() {
            let mut image = [0u16; 1024];
            let mut size = image.len() as u32;
            if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, image.as_mut_ptr(), &mut size)
                != 0
            {
                let full = String::from_utf16_lossy(&image[..size as usize]);
                process = full
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_string();
            }
            CloseHandle(handle);
        }
        let mut rect: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        Some(Foreground {
            title: String::from_utf16_lossy(&title[..len]),
            process,
            bounds: [
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ],
        })
    }
}

#[cfg(not(windows))]
fn foreground() -> Option<Foreground> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, process: &str, x: i32) -> Option<Foreground> {
        Some(Foreground {
            title: title.into(),
            process: process.into(),
            bounds: [x, 0, 800, 600],
        })
    }

    #[test]
    fn a_line_matches_the_windows_contract() {
        let line = ChangeDetector::default()
            .observe(1, 1500, window("Démo", "demo.exe", 0))
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(WINDOWS_LOG_FILE_NAME);
        append(&path, &line).unwrap();
        append(&path, &line).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(
            first,
            serde_json::json!({"segment": 1, "time_ms": 1500, "title": "Démo", "process": "demo.exe", "bounds": [0, 0, 800, 600]})
        );
    }

    #[test]
    fn the_poller_never_starts_off_windows() {
        let tmp = tempfile::tempdir().unwrap();
        drop(spawn(tmp.path(), 0, || Some(Instant::now())));
        assert!(cfg!(windows) || !tmp.path().join(WINDOWS_LOG_FILE_NAME).exists());
    }

    /// Every way a take ends (stop, failed stop, cancel, delete, restart's discard; studio and
    /// instant; each platform's cfg arm) ends its log, or the poller keeps appending to
    /// windows.jsonl after the mirror froze the take. A source scan, because the Windows and macOS
    /// arms do not compile on the Linux test host.
    #[test]
    fn every_stop_and_discard_path_stops_the_log() {
        let source = include_str!("recording.rs");
        let block = &source[source.find("\nimpl ActiveRecording {").unwrap()..];
        let block = &block[..block.find("\n}\n").unwrap()];
        let mut checked = Vec::new();
        let mut lines = block.lines();
        while let Some(line) = lines.next() {
            let Some(name) = line
                .strip_prefix("    ")
                .filter(|l| ["pub", "async", "fn "].iter().any(|p| l.starts_with(p)))
                .and_then(|l| l.split("fn ").nth(1))
                .map(|rest| rest.split(['(', '<']).next().unwrap_or_default())
            else {
                continue;
            };
            let ends_a_take = name == "stop"
                || name.starts_with("cancel_")
                || name.starts_with("discard_")
                || name.ends_with("_stop_handle")
                || name.ends_with("_delete_handle");
            if !ends_a_take {
                continue;
            }
            let body: Vec<_> = lines.by_ref().take_while(|l| *l != "    }").collect();
            assert!(
                body.iter()
                    .any(|l| l.trim() == "crate::window_log::stop(&self.project_dir);"),
                "ActiveRecording::{name} ends a take without crate::window_log::stop"
            );
            checked.push(name);
        }
        for name in [
            "clean_studio_stop_handle",
            "studio_delete_handle",
            "failed_stop_handle",
            "instant_stop_handle",
            "instant_delete_handle",
            "cancel_preserving",
            "cancel_and_delete",
            "discard_cancelled_start",
            "cancel_and_delete_with_query",
            "stop",
        ] {
            assert!(
                checked.contains(&name),
                "scan missed ActiveRecording::{name}"
            );
        }
        // Linux and macOS/Windows each have their own clean_studio_stop_handle.
        let clean = checked.iter().filter(|n| **n == "clean_studio_stop_handle");
        assert_eq!(clean.count(), 2);
    }

    #[test]
    fn pause_ends_the_segment_and_resume_starts_the_next() {
        let tmp = tempfile::tempdir().unwrap();
        let state = || {
            lock()
                .as_ref()
                .map(|current| (current.segment, current.log.is_some()))
        };
        start(tmp.path(), || None);
        assert_eq!(state(), Some((0, true)));
        set_paused(true);
        assert_eq!(state(), Some((0, false)));
        set_paused(false);
        set_paused(false);
        assert_eq!(state(), Some((1, true)));
        stop(&tmp.path().join("an older take"));
        assert_eq!(state(), Some((1, true)));
        stop(tmp.path());
        assert_eq!(state(), None);
        set_paused(false);
        assert_eq!(state(), None);
    }
}
