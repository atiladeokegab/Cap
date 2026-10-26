//! The shot list in the teleprompter (fork): `shotlist.json` in, `shots.json`
//! out, both per the film contracts in `contracts/vendor/0.1.0/`.
//!
//! A shot mark is `{shot_id, segment, time_ms}` on the recording's own clock:
//! segment 0 counts from the start gate's armed instant (the epoch
//! `cursor.json` uses) and every resumed segment from its own resume -- never
//! the bar's elapsed timer, which runs across pauses.
//!
//! Std and serde only: the acceptance driver includes this file by path.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};

/// Beside Cap's files in the take; Cap writes nothing by this name.
pub const SHOTS_FILE: &str = "shots.json";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Line {
    pub sentence_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Shot {
    pub shot_id: String,
    pub title: String,
    #[serde(default)]
    pub on_screen: Option<String>,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShotList {
    /// Read only so a title that is not a string fails the parse.
    #[serde(default, rename = "title")]
    _title: Option<String>,
    pub shots: Vec<Shot>,
}

/// Parses and validates a `shotlist.json`, the rules of
/// `shotlist.schema.json` that serde's types do not already enforce.
pub fn parse(bytes: &[u8]) -> Result<ShotList, String> {
    let list: ShotList =
        serde_json::from_slice(bytes).map_err(|error| format!("shotlist.json: {error}"))?;
    if list.shots.is_empty() {
        return Err("shotlist.json has no shots".into());
    }
    for shot in &list.shots {
        if !is_id(&shot.shot_id) {
            return Err(format!("shotlist.json: bad shot_id {:?}", shot.shot_id));
        }
        if let Some(line) = shot
            .lines
            .iter()
            .find(|line| !is_id(&line.sentence_id) || line.text.is_empty())
        {
            return Err(format!(
                "shotlist.json: bad line {:?} in shot {}",
                line.sentence_id, shot.shot_id
            ));
        }
    }
    Ok(list)
}

/// `^[a-z][a-z0-9_-]*$`, the contracts' id pattern.
fn is_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ShotMark {
    pub shot_id: String,
    pub segment: u32,
    pub time_ms: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retake: bool,
}

/// The clock a mark reads. Pause and resume start a new segment with its own
/// epoch, the way Cap's studio actor starts a new segment pipeline.
#[derive(Default)]
pub struct ShotClock {
    segment: u32,
    resumed_at: Option<Instant>,
    paused: bool,
}

impl ShotClock {
    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self, at: Instant) {
        self.segment += 1;
        self.resumed_at = Some(at);
        self.paused = false;
    }

    /// `(segment, time_ms)` at `now`, given the start gate's armed instant.
    /// `None` while paused or before the gate opens.
    pub fn read(&self, armed: Option<Instant>, now: Instant) -> Option<(u32, u64)> {
        if self.paused {
            return None;
        }
        let epoch = self.resumed_at.or(armed)?;
        Some((
            self.segment,
            now.saturating_duration_since(epoch).as_millis() as u64,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Next,
    Retake,
}

/// The loaded list, the shot on screen, and the marks written into the take.
pub struct Shots {
    pub list: ShotList,
    current: usize,
    started: bool,
    take: Option<PathBuf>,
    marks: Vec<ShotMark>,
}

impl Shots {
    pub fn new(list: ShotList) -> Self {
        Self {
            list,
            current: 0,
            started: false,
            take: None,
            marks: Vec::new(),
        }
    }

    pub fn current(&self) -> &Shot {
        &self.list.shots[self.current]
    }

    pub fn index(&self) -> usize {
        self.current
    }

    pub fn marks(&self) -> &[ShotMark] {
        &self.marks
    }

    /// Next starts the shot on screen the first time, then moves on to the
    /// following one; Retake starts the current shot again. Either way the
    /// mark is appended and the take's `shots.json` rewritten whole.
    pub fn mark(&mut self, action: Action, take: &Path, at: (u32, u64)) -> std::io::Result<()> {
        if self.take.as_deref() != Some(take) {
            // A teleprompter reopened mid-take keeps the marks already written.
            self.marks = std::fs::read(take.join(SHOTS_FILE))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            self.started = !self.marks.is_empty();
            self.take = Some(take.to_path_buf());
        }
        if action == Action::Next && self.started {
            if self.current + 1 == self.list.shots.len() {
                return Ok(());
            }
            self.current += 1;
        }
        self.started = true;
        let (segment, time_ms) = at;
        self.marks.push(ShotMark {
            shot_id: self.current().shot_id.clone(),
            segment,
            time_ms,
            retake: action == Action::Retake,
        });
        let path = take.join(SHOTS_FILE);
        // The mirror uploads this file while recording: never let it read half.
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.marks)?)?;
        std::fs::rename(&temp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn list() -> ShotList {
        parse(
            br#"{"shots": [
                {"shot_id": "intro", "title": "Intro", "lines": [{"sentence_id": "s1", "text": "Hi."}]},
                {"shot_id": "demo", "title": "Demo", "on_screen": "The app", "lines": []}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn rejects_what_the_contract_rejects() {
        for bad in [
            &br#"{}"#[..],
            br#"{"shots": []}"#,
            br#"{"shots": [{"shot_id": 17}]}"#,
            br#"{"shots": [{"shot_id": "Intro", "title": "x", "lines": []}]}"#,
            br#"{"shots": [{"shot_id": "a", "title": "x", "lines": [{"sentence_id": "s1", "text": ""}]}]}"#,
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn next_starts_then_advances_and_stops_at_the_last_shot() {
        let take = tempfile::tempdir().unwrap();
        let mut shots = Shots::new(list());
        for (action, time) in [
            (Action::Next, 10),
            (Action::Retake, 20),
            (Action::Next, 30),
            (Action::Next, 40),
        ] {
            shots.mark(action, take.path(), (0, time)).unwrap();
        }
        let ids: Vec<_> = shots.marks().iter().map(|m| m.shot_id.as_str()).collect();
        assert_eq!(ids, ["intro", "intro", "demo"]);
        assert!(shots.marks()[1].retake);
        let written: Vec<ShotMark> =
            serde_json::from_slice(&std::fs::read(take.path().join(SHOTS_FILE)).unwrap()).unwrap();
        assert_eq!(written, shots.marks());
    }

    #[test]
    fn a_resumed_segment_counts_from_its_own_resume() {
        let armed = Instant::now();
        let mut clock = ShotClock::default();
        assert_eq!(clock.read(None, armed), None);
        assert_eq!(
            clock.read(Some(armed), armed + Duration::from_millis(900)),
            Some((0, 900))
        );
        clock.pause();
        assert_eq!(
            clock.read(Some(armed), armed + Duration::from_secs(2)),
            None
        );
        clock.resume(armed + Duration::from_secs(5));
        assert_eq!(
            clock.read(Some(armed), armed + Duration::from_millis(5150)),
            Some((1, 150))
        );
    }
}
