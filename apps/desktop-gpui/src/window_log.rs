//! The window log (design D18): while a take records, poll the foreground window every 250 ms and
//! append one line to the take's `windows.jsonl` each time it changes. Times are on the take's
//! start-gate clock, the same epoch `cursor.json` uses, so a title lines up with the clicks under it.
//! Only Windows has a poller; other platforms compile it out and log nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

pub const WINDOWS_LOG_FILE_NAME: &str = "windows.jsonl";
const POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Foreground {
    pub title: String,
    pub process: String,
    /// x, y, width, height in screen pixels
    pub bounds: [i32; 4],
}

#[derive(Debug, PartialEq, Serialize)]
pub struct WindowLine {
    pub segment: u32,
    pub time_ms: u64,
    #[serde(flatten)]
    pub window: Foreground,
}

/// Emits a line only when the window's title or process changes; a move or resize alone is not a change.
#[derive(Default)]
pub struct ChangeDetector {
    last: Option<(String, String)>,
}

impl ChangeDetector {
    pub fn observe(
        &mut self,
        segment: u32,
        time_ms: u64,
        window: Option<Foreground>,
    ) -> Option<WindowLine> {
        let window = window?;
        let key = (window.title.clone(), window.process.clone());
        if self.last.as_ref() == Some(&key) {
            return None;
        }
        self.last = Some(key);
        Some(WindowLine {
            segment,
            time_ms,
            window,
        })
    }
}

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

fn poll(path: PathBuf, segment: u32, epoch: impl Fn() -> Option<Instant>, stop: Arc<AtomicBool>) {
    let mut detector = ChangeDetector::default();
    while !stop.load(Ordering::Relaxed) {
        if let Some(epoch) = epoch() {
            let time_ms = Instant::now().saturating_duration_since(epoch).as_millis() as u64;
            if let Some(line) = detector.observe(segment, time_ms, foreground()) {
                if let Err(e) = append(&path, &line) {
                    tracing::warn!("window log write failed: {e}");
                }
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
    fn writes_a_line_only_when_the_foreground_window_changes() {
        let mut detector = ChangeDetector::default();
        let seen: Vec<_> = [
            (0, window("Demo", "demo.exe", 0)),
            (250, window("Demo", "demo.exe", 40)),
            (500, None),
            (750, window("Result", "demo.exe", 0)),
            (1000, window("Result", "other.exe", 0)),
            (1250, window("Result", "other.exe", 0)),
        ]
        .into_iter()
        .filter_map(|(t, w)| detector.observe(0, t, w))
        .map(|line| (line.time_ms, line.window.title, line.window.process))
        .collect();
        assert_eq!(
            seen,
            [
                (0, "Demo".to_string(), "demo.exe".to_string()),
                (750, "Result".into(), "demo.exe".into()),
                (1000, "Result".into(), "other.exe".into())
            ]
        );
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
}
