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
/// The list a take's marks were made against, beside `shots.json`: a later
/// republished list (pick-ups) never changes what an old take's marks mean.
pub const TAKE_LIST_FILE: &str = "shotlist.json";

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
    #[serde(
        default,
        deserialize_with = "present_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub on_screen: Option<String>,
    pub lines: Vec<Line>,
    /// `"none"` turns generated zoom off over this shot.
    #[serde(
        default,
        deserialize_with = "present_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub zoom: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ShotList {
    /// Read only so a title that is not a string fails the parse.
    #[serde(
        default,
        rename = "title",
        deserialize_with = "present_string",
        skip_serializing_if = "Option::is_none"
    )]
    _title: Option<String>,
    pub shots: Vec<Shot>,
    /// The film's what's-left checklist, checked apart from the shots so a
    /// malformed one never blocks them. A `null` one is kept, so it warns.
    #[serde(
        default,
        rename = "progress",
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    raw_progress: Option<serde_json::Value>,
    #[serde(skip)]
    pub progress: Option<Progress>,
    /// Why `progress` was ignored, one line.
    #[serde(skip)]
    pub warning: Option<String>,
}

/// What's left: shots with a good take (of the full list, not a pick-up
/// list's), and each judging criterion covered or not.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Progress {
    pub shots: Vec<ShotDone>,
    pub criteria: Vec<Criterion>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ShotDone {
    pub shot_id: String,
    pub title: String,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub name: String,
    pub covered: bool,
    /// No evidence either way on the film side: never ticked.
    #[serde(default)]
    pub unverified: bool,
}

impl Progress {
    /// `N of M shots done`.
    pub fn summary(&self) -> String {
        let done = self.shots.iter().filter(|shot| shot.done).count();
        format!("{done} of {} shots done", self.shots.len())
    }

    /// One `(label, ticked)` row per shot, then per criterion.
    pub fn rows(&self) -> Vec<(String, bool)> {
        let shots = self.shots.iter().map(|s| (s.title.clone(), s.done));
        let criteria = self.criteria.iter().map(|c| {
            let label = match c.unverified {
                true => format!("{} (unverified)", c.name),
                false => c.name.clone(),
            };
            (label, c.covered && !c.unverified)
        });
        shots.chain(criteria).collect()
    }
}

/// The largest list the teleprompter reads: it comes from the bucket,
/// untrusted, and a real one is a few KiB.
pub const MAX_LIST_BYTES: usize = 1 << 20;

/// The contract lets an optional string be left out, never be `null`.
fn present_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

/// Any value that is there, `null` included.
fn present_value<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(d).map(Some)
}

/// Parses and validates a `shotlist.json`, the rules of
/// `shotlist.schema.json` that serde's types do not already enforce.
pub fn parse(bytes: &[u8]) -> Result<ShotList, String> {
    if bytes.len() > MAX_LIST_BYTES {
        return Err(format!(
            "shotlist.json is over {} KiB",
            MAX_LIST_BYTES >> 10
        ));
    }
    let mut list: ShotList =
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
    if let Some(raw) = list.raw_progress.take() {
        match serde_json::from_value(raw) {
            Ok(progress) => list.progress = Some(progress),
            Err(error) => list.warning = Some(format!("shotlist.json progress ignored: {error}")),
        }
    }
    Ok(list)
}

/// Shot-list reads in flight: each read takes a number when it starts, and
/// a read that lands after a newer one has is dropped, so a slow old read
/// never replaces a newer list.
#[derive(Default)]
pub struct Reads {
    started: u64,
    landed: u64,
}

impl Reads {
    pub fn start(&mut self) -> u64 {
        self.started += 1;
        self.started
    }

    /// Whether read `number`'s result is still the newest to land.
    pub fn land(&mut self, number: u64) -> bool {
        let newest = number > self.landed;
        self.landed = self.landed.max(number);
        newest
    }
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

/// Between shots: what the teleprompter shows over the shot's lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cue {
    #[default]
    Reading,
    /// Scrolled past the shot's last line.
    Stop,
    /// Recording paused: the card of the shot the action starts, and Go.
    Ready(Action),
    /// Go pressed: 3-2-1, then the recording resumes and the action marks.
    Countdown(Action),
}

/// Go's 3-2-1, one count a second.
pub const COUNTDOWN_MS: u64 = 3000;

/// The card's title against the script's text size: large, read at a glance.
pub const CARD_SCALE: f32 = 1.5;

/// The count Go's 3-2-1 shows `elapsed_ms` after Go; `None` once it is over.
pub fn countdown(elapsed_ms: u64) -> Option<u64> {
    (elapsed_ms < COUNTDOWN_MS).then(|| (COUNTDOWN_MS - elapsed_ms).div_ceil(1000))
}

/// Where `shots.json` puts each `zoom: none` shot: `(segment, from_ms,
/// to_ms)`, up to the next mark in its segment or the segment's end (`None`).
pub fn zoomless(list: &ShotList, marks: &[ShotMark]) -> Vec<(u32, u64, Option<u64>)> {
    marks
        .iter()
        .enumerate()
        .filter(|(_, mark)| {
            list.shots
                .iter()
                .any(|shot| shot.shot_id == mark.shot_id && shot.zoom.as_deref() == Some("none"))
        })
        .map(|(index, mark)| {
            let to = marks
                .get(index + 1)
                .filter(|next| next.segment == mark.segment)
                .map(|next| next.time_ms);
            (mark.segment, mark.time_ms, to)
        })
        .collect()
}

/// [`zoomless`] for a take on disk: its marks against the list recorded
/// with them. `live` is only asked for a take marked before takes recorded
/// their list.
pub fn take_zoomless(
    take: &Path,
    live: impl FnOnce() -> Option<ShotList>,
) -> Vec<(u32, u64, Option<u64>)> {
    let marks: Vec<ShotMark> = std::fs::read(take.join(SHOTS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if marks.is_empty() {
        return Vec::new();
    }
    std::fs::read(take.join(TAKE_LIST_FILE))
        .ok()
        .and_then(|bytes| parse(&bytes).ok())
        .or_else(live)
        .map_or_else(Vec::new, |list| zoomless(&list, &marks))
}

/// The mirror uploads take files while recording: never let it read half.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

/// The loaded list, the shot on screen, and the marks written into the take.
pub struct Shots {
    pub list: ShotList,
    current: usize,
    started: bool,
    take: Option<PathBuf>,
    marks: Vec<ShotMark>,
    pub cue: Cue,
    /// A list read mid-take, shown once the take ends.
    pending: Option<ShotList>,
}

impl Shots {
    pub fn new(list: ShotList) -> Self {
        Self {
            list,
            current: 0,
            started: false,
            take: None,
            marks: Vec::new(),
            cue: Cue::Reading,
            pending: None,
        }
    }

    /// A newly read list: shown from its first shot before the take's first
    /// mark; once a shot is under way it waits for the take to end, so the
    /// shot on screen never moves under the reader. A read started during a
    /// live take (Reload, not the take's own read at its start) waits too,
    /// even before the first mark: the take keeps the list it started with.
    pub fn replace(&mut self, list: ShotList, during_take: bool) {
        if self.started || (during_take && self.take.is_some()) {
            self.pending = Some(list);
        } else {
            self.list = list;
            self.current = 0;
            self.cue = Cue::Reading;
            self.pending = None;
        }
    }

    /// The shot a Ready or Countdown cue's action starts, for its card.
    pub fn upcoming(&self) -> Option<&Shot> {
        match self.cue {
            Cue::Ready(Action::Next) | Cue::Countdown(Action::Next) => {
                self.list.shots.get(self.current + 1)
            }
            Cue::Ready(Action::Retake) | Cue::Countdown(Action::Retake) => Some(self.current()),
            _ => None,
        }
    }

    /// The teleprompter scrolled past the shot's last line.
    pub fn ended(&mut self) {
        if self.started && self.cue == Cue::Reading {
            self.cue = Cue::Stop;
        }
    }

    /// Next shot on a started take: stop between shots on the next one's card
    /// (the caller pauses the recording); Retake on a card switches it to
    /// this shot's. False when the press marks at once instead: the take's
    /// first shot, the last shot, or a Retake while recording.
    pub fn hold(&mut self, action: Action) -> bool {
        let hold = match (action, self.cue) {
            // A press during the 3-2-1 changes nothing.
            (_, Cue::Countdown(_)) => return true,
            (Action::Next, _) => self.started && self.current + 1 < self.list.shots.len(),
            (Action::Retake, cue) => matches!(cue, Cue::Ready(_)),
        };
        if hold {
            self.cue = Cue::Ready(action);
        }
        hold
    }

    /// Go on a Ready card: the action the resume will mark.
    pub fn go(&mut self) -> Option<Action> {
        let Cue::Ready(action) = self.cue else {
            return None;
        };
        self.cue = Cue::Countdown(action);
        Some(action)
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
                if let Some(list) = self.pending.take() {
                    self.list = list;
                }
                self.current = 0;
                self.started = false;
                self.take = None;
                self.marks.clear();
                self.cue = Cue::Reading;
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
        self.cue = Cue::Reading;
        let (segment, time_ms) = at;
        self.marks.push(ShotMark {
            shot_id: self.current().shot_id.clone(),
            segment,
            time_ms,
            retake: action == Action::Retake,
        });
        // The list first: marks are never on disk without the list they mean.
        write_whole(
            &take.join(TAKE_LIST_FILE),
            &serde_json::to_vec_pretty(&self.list)?,
        )?;
        write_whole(
            &take.join(SHOTS_FILE),
            &serde_json::to_vec_pretty(&self.marks)?,
        )
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
    fn next_shot_holds_on_the_next_card_and_go_marks_at_the_resume() {
        let take = std::env::temp_dir().join(format!("cap-shots-cue-{}", std::process::id()));
        std::fs::create_dir_all(&take).unwrap();
        let mut shots = Shots::new(list());
        // The take's first press starts shot 1 at once; nothing to stop yet.
        shots.ended();
        assert_eq!(shots.cue, Cue::Reading);
        assert!(!shots.hold(Action::Next));
        shots.mark(Action::Next, &take, (0, 100)).unwrap();
        shots.ended();
        assert_eq!(shots.cue, Cue::Stop);
        assert!(shots.hold(Action::Next));
        assert_eq!(shots.upcoming().unwrap().shot_id, "demo");
        // Retake on the card: this shot's card instead, and back.
        assert!(shots.hold(Action::Retake));
        assert_eq!(shots.upcoming().unwrap().shot_id, "intro");
        assert!(shots.hold(Action::Next));
        assert_eq!(shots.go(), Some(Action::Next));
        assert!(shots.hold(Action::Retake));
        assert_eq!(shots.cue, Cue::Countdown(Action::Next));
        assert_eq!(shots.go(), None);
        let shown: Vec<_> = [0, 999, 1000, 2000, 2999, 3000].map(countdown).to_vec();
        assert_eq!(shown, [Some(3), Some(3), Some(2), Some(1), Some(1), None]);
        shots.mark(Action::Next, &take, (1, 0)).unwrap();
        assert_eq!(
            (shots.cue, shots.current().shot_id.as_str()),
            (Cue::Reading, "demo")
        );
        // Recording: Retake marks at once; the last shot has no next card.
        assert!(!shots.hold(Action::Retake));
        assert!(!shots.hold(Action::Next));
        assert_eq!(shots.upcoming(), None);
        std::fs::remove_dir_all(&take).unwrap();
    }

    #[test]
    fn zoomless_spans_run_from_the_mark_to_the_next_in_the_segment() {
        let list = parse(
            br#"{"shots": [
                {"shot_id": "intro", "title": "I", "lines": [], "zoom": "none"},
                {"shot_id": "demo", "title": "D", "lines": []}
            ]}"#,
        )
        .unwrap();
        assert!(
            parse(br#"{"shots": [{"shot_id": "a", "title": "x", "lines": [], "zoom": null}]}"#)
                .is_err()
        );
        let mark = |shot_id: &str, segment, time_ms| ShotMark {
            shot_id: shot_id.into(),
            segment,
            time_ms,
            retake: false,
        };
        let marks = [
            mark("intro", 0, 100),
            mark("demo", 0, 900),
            mark("intro", 1, 0),
            mark("demo", 2, 0),
        ];
        assert_eq!(zoomless(&list, &marks), [(0, 100, Some(900)), (1, 0, None)]);
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

    #[test]
    fn a_list_read_mid_take_waits_for_the_take_to_end_and_late_reads_drop() {
        let take = std::env::temp_dir().join(format!("cap-shots-reload-{}", std::process::id()));
        std::fs::create_dir_all(&take).unwrap();
        let pickup =
            || parse(br#"{"shots": [{"shot_id": "ask", "title": "Ask", "lines": []}]}"#).unwrap();
        let mut shots = Shots::new(list());
        shots.follow(Some(&take));
        shots.mark(Action::Next, &take, (0, 100)).unwrap();
        shots.mark(Action::Next, &take, (0, 200)).unwrap();
        shots.replace(pickup(), true);
        assert_eq!(shots.current().shot_id, "demo");
        shots.follow(None);
        assert_eq!(shots.current().shot_id, "ask");
        // Between takes a new list shows at once, from its first shot.
        shots.replace(list(), true);
        assert_eq!(shots.current().shot_id, "intro");
        std::fs::remove_dir_all(&take).unwrap();

        let mut reads = Reads::default();
        let (old, new) = (reads.start(), reads.start());
        assert!(reads.land(new));
        assert!(!reads.land(old));
    }

    #[test]
    fn a_reload_before_the_first_mark_keeps_the_list_the_take_started_with() {
        let take = std::env::temp_dir().join(format!("cap-shots-early-{}", std::process::id()));
        let ask =
            || parse(br#"{"shots": [{"shot_id": "ask", "title": "Ask", "lines": []}]}"#).unwrap();
        let mut shots = Shots::new(list());
        shots.follow(Some(&take));
        // The take's own read at its start shows at once.
        shots.replace(ask(), false);
        assert_eq!(shots.current().shot_id, "ask");
        // Reload before the first mark waits for the take to end.
        shots.replace(list(), true);
        assert_eq!(shots.current().shot_id, "ask");
        shots.follow(None);
        assert_eq!(shots.current().shot_id, "intro");
    }

    #[test]
    fn a_take_resolves_zoom_none_from_the_list_it_was_marked_against() {
        let take = std::env::temp_dir().join(format!("cap-shots-zoomless-{}", std::process::id()));
        std::fs::create_dir_all(&take).unwrap();
        let recorded = parse(
            br#"{"shots": [
                {"shot_id": "intro", "title": "I", "on_screen": "Terminal", "lines": [], "zoom": "none"},
                {"shot_id": "demo", "title": "D", "lines": []}
            ]}"#,
        )
        .unwrap();
        // The pick-ups republished since drop the opt-out.
        let pickups = || {
            Some(parse(br#"{"shots": [{"shot_id": "intro", "title": "I", "lines": []}]}"#).unwrap())
        };
        assert!(take_zoomless(&take, pickups).is_empty());
        let mut shots = Shots::new(recorded);
        shots.follow(Some(&take));
        shots.mark(Action::Next, &take, (0, 700)).unwrap();
        assert_eq!(take_zoomless(&take, pickups), [(0, 700, None)]);
        // A take from before takes kept their list falls back to the live one.
        std::fs::remove_file(take.join(TAKE_LIST_FILE)).unwrap();
        assert!(take_zoomless(&take, pickups).is_empty());
        std::fs::remove_dir_all(&take).unwrap();
    }

    #[test]
    fn progress_is_read_apart_and_a_bad_one_is_only_a_warning() {
        let with = |progress: &str| {
            parse(
                format!(
                    r#"{{"shots": [{{"shot_id": "a", "title": "A", "lines": []}}], "progress": {progress}}}"#
                )
                .as_bytes(),
            )
            .unwrap()
        };
        let good = with(
            r#"{"shots": [{"shot_id": "a", "title": "A", "done": true}, {"shot_id": "b", "title": "B", "done": false}],
                "criteria": [{"id": "wow", "name": "Wow", "covered": true}, {"id": "p", "name": "Pitch", "covered": true, "unverified": true}]}"#,
        );
        let progress = good.progress.unwrap();
        assert_eq!(progress.summary(), "1 of 2 shots done");
        assert_eq!(
            progress.rows(),
            [
                ("A".into(), true),
                ("B".into(), false),
                ("Wow".into(), true),
                ("Pitch (unverified)".into(), false)
            ]
        );
        assert!(good.warning.is_none());
        for bad in [
            "null",
            r#""1 of 3""#,
            r#"{"shots": [], "criteria": [{"id": "w", "name": "W"}]}"#,
        ] {
            let list = with(bad);
            assert!(list.progress.is_none());
            assert!(!list.warning.unwrap().contains('\n'));
        }
        let huge = format!(
            r#"{{"title": "{}", "shots": [{{"shot_id": "a", "title": "A", "lines": []}}]}}"#,
            "x".repeat(MAX_LIST_BYTES)
        );
        assert!(parse(huge.as_bytes()).is_err());
    }
}
