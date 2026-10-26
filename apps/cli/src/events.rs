//! `cap events <take> --json`: every cursor move, click and key press of a take, per segment, on the
//! take's start-gate clock (the cursor.json clock). The film pipeline reads keyboard.bin through this
//! instead of decoding Cap's binary format itself. Output follows the film repo's cap-events contract.

use std::path::{Path, PathBuf};

use cap_project::{CursorEvents, KeyboardEvents};
use clap::Args;
use serde::Serialize;

#[derive(Args)]
pub struct EventsArgs {
    /// Path to a '.cap' project
    path: PathBuf,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct InputEvent {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    segment: u32,
    time_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    down: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_code: Option<String>,
}

impl InputEvent {
    fn new(kind: &'static str, segment: u32, time_ms: f64) -> Self {
        Self {
            id: String::new(),
            kind,
            segment,
            time_ms,
            x: None,
            y: None,
            down: None,
            cursor_id: None,
            key: None,
            key_code: None,
        }
    }
}

impl EventsArgs {
    pub fn run(self) -> Result<(), String> {
        crate::write_json(&take_events(&self.path)?)
    }
}

fn segment_dirs(take: &Path) -> Result<Vec<(u32, PathBuf)>, String> {
    let root = take.join("content/segments");
    let entries =
        std::fs::read_dir(&root).map_err(|e| format!("No segments in {}: {e}", root.display()))?;
    let mut dirs: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            Some((name.strip_prefix("segment-")?.parse().ok()?, entry.path()))
        })
        .collect();
    dirs.sort_by_key(|(index, _)| *index);
    Ok(dirs)
}

/// Reads only; a take without cursor.json or keyboard.bin in a segment just has no such events there.
pub fn take_events(take: &Path) -> Result<Vec<InputEvent>, String> {
    let mut events = Vec::new();
    for (segment, dir) in segment_dirs(take)? {
        let cursor = dir.join("cursor.json");
        if cursor.is_file() {
            let cursor = CursorEvents::load_from_file(&cursor)
                .map_err(|e| format!("segment {segment}: {e}"))?;
            events.extend(cursor.moves.into_iter().map(|m| InputEvent {
                x: Some(m.x),
                y: Some(m.y),
                cursor_id: Some(m.cursor_id),
                ..InputEvent::new("move", segment, m.time_ms)
            }));
            events.extend(cursor.clicks.into_iter().map(|c| InputEvent {
                down: Some(c.down),
                cursor_id: Some(c.cursor_id),
                ..InputEvent::new("click", segment, c.time_ms)
            }));
        }
        let keyboard = dir.join(cap_project::KEYBOARD_EVENTS_FILE_NAME);
        if keyboard.is_file() {
            events.extend(
                KeyboardEvents::load_from_file(&keyboard)
                    .map_err(|e| format!("segment {segment}: {e}"))?
                    .presses
                    .into_iter()
                    .map(|k| InputEvent {
                        down: Some(k.down),
                        key: Some(k.key),
                        key_code: Some(k.key_code),
                        ..InputEvent::new("key", segment, k.time_ms)
                    }),
            );
        }
    }
    events.sort_by(|a, b| {
        (a.segment, a.time_ms)
            .partial_cmp(&(b.segment, b.time_ms))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Ids number each kind in take order, so a rerun on the same take gives the same ids.
    let mut counts = std::collections::HashMap::new();
    for event in &mut events {
        let n = counts.entry(event.kind).or_insert(0);
        *n += 1;
        event.id = format!("{}-{n}", event.kind);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_project::KeyPressEvent;

    #[test]
    fn reads_moves_clicks_and_keys_per_segment_in_time_order() {
        let take = tempfile::tempdir().unwrap();
        for (segment, offset) in [(0, 0.0), (1, 100.0)] {
            let dir = take
                .path()
                .join(format!("content/segments/segment-{segment}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("cursor.json"),
                serde_json::json!({
                    "moves": [{"active_modifiers": [], "cursor_id": "0", "time_ms": 500.0 + offset, "x": 0.5, "y": 0.25}],
                    "clicks": [{"active_modifiers": [], "cursor_num": 0, "cursor_id": "0", "time_ms": 600.5 + offset, "down": true}]
                })
                .to_string(),
            )
            .unwrap();
            KeyboardEvents {
                presses: vec![KeyPressEvent {
                    key: "a".into(),
                    key_code: "KeyA".into(),
                    time_ms: 750.0 + offset,
                    down: true,
                }],
            }
            .write_to_file(&dir.join("keyboard.bin"))
            .unwrap();
        }
        let events = take_events(take.path()).unwrap();
        let got: Vec<_> = events
            .iter()
            .map(|e| (e.id.as_str(), e.segment, e.time_ms))
            .collect();
        assert_eq!(
            got,
            [
                ("move-1", 0, 500.0),
                ("click-1", 0, 600.5),
                ("key-1", 0, 750.0),
                ("move-2", 1, 600.0),
                ("click-2", 1, 700.5),
                ("key-2", 1, 850.0)
            ]
        );
        assert_eq!(events[2].key.as_deref(), Some("a"));
        assert_eq!(events[2].key_code.as_deref(), Some("KeyA"));
        let json = serde_json::to_value(&events[1]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"id": "click-1", "type": "click", "segment": 0, "time_ms": 600.5, "down": true, "cursor_id": "0"})
        );
    }
}
