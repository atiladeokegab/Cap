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
/// Where the film's story step publishes the list, at the takes bucket's root.
pub const BUCKET_KEY: &str = "shotlist/current.json";
/// The offline fallback, in the app-data dir.
pub const LOCAL_FILE: &str = "shotlist.json";

/// The bucket's list wins; offline, missing or malformed there, the local
/// file. Returns the list, if any (none is the stock teleprompter), and a
/// warning line for the teleprompter.
pub fn pick(
    bucket: Result<Vec<u8>, String>,
    local: impl FnOnce() -> std::io::Result<Vec<u8>>,
) -> (Option<ShotList>, Option<String>) {
    let mut warnings = Vec::new();
    if let Ok(bytes) = bucket {
        match parse(&bytes) {
            Ok(list) => return (Some(list), None),
            Err(error) => warnings.push(format!("bucket {error}")),
        }
    }
    let list = match local() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            warnings.push(format!("{LOCAL_FILE}: {error}"));
            None
        }
        Ok(bytes) => parse(&bytes)
            .inspect_err(|error| warnings.push(error.clone()))
            .ok(),
    };
    (list, (!warnings.is_empty()).then(|| warnings.join("; ")))
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Line {
    pub sentence_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Shot {
    pub shot_id: String,
    pub title: String,
    #[serde(default, deserialize_with = "present_string")]
    pub on_screen: Option<String>,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShotList {
    /// Read only so a title that is not a string fails the parse.
    #[serde(default, rename = "title", deserialize_with = "present_string")]
    _title: Option<String>,
    pub shots: Vec<Shot>,
}

/// The contract lets an optional string be left out, never be `null`.
fn present_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
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

    /// `at` is the resumed segment's epoch as the recording actor reports
    /// it. Without one, marks wait for the next resume rather than guess.
    pub fn resume(&mut self, at: Option<Instant>) {
        self.segment += 1;
        self.resumed_at = at;
        self.paused = at.is_none();
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

    /// The live take's saved marks, and the shot they leave on screen: a
    /// teleprompter reopened mid-take picks up where it left off.
    pub fn open(&mut self, take: &Path) {
        if self.take.as_deref() != Some(take) {
            // A teleprompter reopened mid-take keeps the marks already written.
            self.marks = std::fs::read(take.join(SHOTS_FILE))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            self.started = !self.marks.is_empty();
            // The last mark's shot is on screen again (a retake did not move
            // on); a new, empty take starts at the first shot.
            self.current = self
                .marks
                .last()
                .and_then(|last| {
                    self.list
                        .shots
                        .iter()
                        .position(|shot| shot.shot_id == last.shot_id)
                })
                .unwrap_or(0);
            self.take = Some(take.to_path_buf());
        }
    }

    /// The session's live take, followed as it changes so the shot on screen
    /// is always the one a press would mark: a take's own place in the list,
    /// and between takes (no take) the first shot of the next.
    pub fn follow(&mut self, take: Option<&Path>) {
        match take {
            Some(take) => self.open(take),
            None => {
                self.current = 0;
                self.started = false;
                self.take = None;
                self.marks.clear();
            }
        }
    }

    /// Next starts the shot on screen the first time, then moves on to the
    /// following one; Retake starts the current shot again. Either way the
    /// mark is appended and the take's `shots.json` rewritten whole.
    pub fn mark(&mut self, action: Action, take: &Path, at: (u32, u64)) -> std::io::Result<()> {
        self.open(take);
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
            br#"{"title": null, "shots": [{"shot_id": "a", "title": "x", "lines": []}]}"#,
            br#"{"shots": [{"shot_id": "a", "title": "x", "on_screen": null, "lines": []}]}"#,
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn the_bucket_list_wins_and_the_local_file_is_the_fallback() {
        let one = |id: &str| {
            format!(r#"{{"shots": [{{"shot_id": "{id}", "title": "T", "lines": []}}]}}"#)
                .into_bytes()
        };
        let first = |(list, warning): (Option<ShotList>, Option<String>)| {
            (list.map(|list| list.shots[0].shot_id.clone()), warning)
        };
        let missing = || Err(std::io::ErrorKind::NotFound.into());
        assert_eq!(
            first(pick(Ok(one("bucket")), || Ok(one("local")))),
            (Some("bucket".into()), None)
        );
        assert_eq!(
            first(pick(Err("404".into()), || Ok(one("local")))),
            (Some("local".into()), None)
        );
        assert_eq!(first(pick(Err("offline".into()), missing)), (None, None));
        // Malformed in the bucket: the local file, and say so.
        let (list, warning) = first(pick(Ok(b"{}".to_vec()), || Ok(one("local"))));
        assert_eq!(list.as_deref(), Some("local"));
        assert!(warning.unwrap().starts_with("bucket shotlist.json"));
        assert!(pick(Ok(b"{}".to_vec()), missing).1.is_some());
    }

    #[test]
    fn a_reopened_teleprompter_resumes_at_the_last_marked_shot() {
        let three = || {
            parse(
                br#"{"shots": [
                {"shot_id": "intro", "title": "I", "lines": []},
                {"shot_id": "demo", "title": "D", "lines": []},
                {"shot_id": "ask", "title": "A", "lines": []}
            ]}"#,
            )
            .unwrap()
        };
        let dir = std::env::temp_dir().join(format!("cap-shots-reopen-{}", std::process::id()));
        let (take, fresh) = (dir.join("take"), dir.join("fresh"));
        std::fs::create_dir_all(&take).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        let mut first = Shots::new(three());
        for (action, at) in [
            (Action::Next, 100),
            (Action::Next, 200),
            (Action::Retake, 300),
        ] {
            first.mark(action, &take, (0, at)).unwrap();
        }
        // Reopened twice: Retake re-marks demo, then Next moves on to ask.
        let mut reopened = Shots::new(three());
        reopened.mark(Action::Retake, &take, (0, 400)).unwrap();
        assert_eq!(reopened.current().shot_id, "demo");
        let mut again = Shots::new(three());
        again.mark(Action::Next, &take, (0, 500)).unwrap();
        let ids: Vec<_> = again.marks().iter().map(|m| m.shot_id.as_str()).collect();
        assert_eq!(ids, ["intro", "demo", "demo", "demo", "ask"]);
        // A new, empty take starts at the first shot again.
        again.mark(Action::Next, &fresh, (0, 0)).unwrap();
        assert_eq!(again.current().shot_id, "intro");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_take_shows_its_first_shot_before_any_mark() {
        let dir = std::env::temp_dir().join(format!("cap-shots-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut shots = Shots::new(list());
        shots.follow(Some(&dir));
        shots.mark(Action::Next, &dir, (0, 10)).unwrap();
        shots.mark(Action::Next, &dir, (0, 20)).unwrap();
        assert_eq!(shots.current().shot_id, "demo");
        // The take stops: the window shows shot 1 for the next take...
        shots.follow(None);
        assert_eq!(
            (shots.current().shot_id.as_str(), shots.marks().len()),
            ("intro", 0)
        );
        // ...and the same take, still live, keeps its place.
        shots.follow(Some(&dir));
        assert_eq!(shots.current().shot_id, "demo");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn next_starts_then_advances_and_stops_at_the_last_shot() {
        // Std only: the acceptance driver's crate compiles these tests too.
        let take = std::env::temp_dir().join(format!("cap-shots-{}", std::process::id()));
        std::fs::create_dir_all(&take).unwrap();
        let mut shots = Shots::new(list());
        for (action, time) in [
            (Action::Next, 10),
            (Action::Retake, 20),
            (Action::Next, 30),
            (Action::Next, 40),
        ] {
            shots.mark(action, &take, (0, time)).unwrap();
        }
        let ids: Vec<_> = shots.marks().iter().map(|m| m.shot_id.as_str()).collect();
        assert_eq!(ids, ["intro", "intro", "demo"]);
        assert!(shots.marks()[1].retake);
        let written: Vec<ShotMark> =
            serde_json::from_slice(&std::fs::read(take.join(SHOTS_FILE)).unwrap()).unwrap();
        assert_eq!(written, shots.marks());
        std::fs::remove_dir_all(&take).unwrap();
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
        clock.resume(Some(armed + Duration::from_secs(5)));
        assert_eq!(
            clock.read(Some(armed), armed + Duration::from_millis(5150)),
            Some((1, 150))
        );
        // No epoch from the actor: no mark, never the armed clock's time.
        clock.pause();
        clock.resume(None);
        assert_eq!(
            clock.read(Some(armed), armed + Duration::from_secs(9)),
            None
        );
    }
}
