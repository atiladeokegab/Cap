//! The window log's pure part: what counts as a change and where a line falls on the take's clock.
//! No platform or app types, so the acceptance driver (`gcs-mirror/src/bin/cap-driver.rs`) includes
//! this same file with `#[path]` and runs the code the app runs.

use std::time::Instant;

use serde::Serialize;

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

/// Milliseconds from the segment's start-gate epoch (the clock cursor.json uses) to `at`.
pub fn time_ms(epoch: Instant, at: Instant) -> u64 {
    at.saturating_duration_since(epoch).as_millis() as u64
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
    fn time_is_on_the_epoch_and_never_before_it() {
        let epoch = Instant::now();
        assert_eq!(time_ms(epoch, epoch + Duration::from_millis(850)), 850);
        assert_eq!(time_ms(epoch + Duration::from_millis(5), epoch), 0);
    }
}
