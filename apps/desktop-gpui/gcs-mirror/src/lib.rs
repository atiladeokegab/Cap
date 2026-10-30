//! Streams a Studio take to a GCS bucket while it records (design D10, D21).
//!
//! Nothing Cap wrote is ever changed: closed fragments are hard-linked into
//! the outbox and every other file is copied there at stop, so Cap's remux
//! deleting its fragment folders loses no upload input. `uploaded.jsonl` in
//! the outbox records each accepted object, so a restart resumes without
//! re-sending one, and `take-manifest.json` goes last.

pub mod sigv4;

use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

/// The film repo's contracts this fork writes (vendored, D23).
pub const CONTRACTS_VERSION: &str = "0.1.0";
/// The pre-remux Studio layout: `content/segments/segment-N/{display,camera}`
/// m4s fragments with manifests, per-segment audio, cursor and keyboard files.
pub const PROJECT_FORMAT_VERSION: u32 = 1;
pub const FORK_COMMIT: &str = env!("CAP_FORK_COMMIT");
const AUDIO_DELTA: u64 = 256 * 1024;
const STOP: &str = "stop.json";
const MANIFEST: &str = "take-manifest.json";
const LEDGER: &str = "uploaded.jsonl";
/// Bytes sent of the frozen take, PUT while it drains so the VM can tell a
/// slow upload from a dead one. Never in the ledger, `stop.json` or the manifest.
const PROGRESS: &str = "progress.json";
const PROGRESS_EVERY: Duration = Duration::from_secs(15);
/// A take file's PUT fails once no bytes have moved for this long, however
/// long the whole upload runs.
const STALL: Duration = Duration::from_secs(60);
/// 1 Mbit/s: a take file's PUT may take as long as this uplink would need.
const BYTES_PER_SECOND_FLOOR: u64 = 125_000;
/// Written instead of `stop.json` when the freeze fails.
const FAILED: &str = "mirror-failed.json";
/// The presenter's Send (film's send.json contract): `stop.json` says
/// `send_required`, so the film watcher builds nothing until this lands. A
/// control file, kept in the outbox and never listed in `stop.json`.
const SEND: &str = "send.json";
pub const CAP_SEND_VERSION: &str = "1";
/// Local only: the presenter discarded the take, so nothing of it is sent
/// again and what went up is deleted.
const DISCARDED: &str = "discarded.json";

pub fn discarded(outbox: &Path) -> bool {
    outbox.join(DISCARDED).is_file()
}

/// A take stopped by this fork that is neither sent nor discarded. Takes from
/// a fork before the choice (no `send_required`) are never asked about.
pub fn undecided(outbox: &Path) -> bool {
    !outbox.join(SEND).is_file()
        && !discarded(outbox)
        && fs::read(outbox.join(STOP))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok())
            .is_some_and(|stop| stop["send_required"] == true)
}

/// The local config file in the app data folder; the mirror is off without it.
#[derive(Clone, Deserialize)]
pub struct Config {
    pub bucket: String,
    #[serde(alias = "hmac_access_id")]
    pub access_key: String,
    #[serde(alias = "hmac_secret")]
    pub secret_key: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
}

fn default_region() -> String {
    "auto".into()
}

impl Config {
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read(path).ok()?;
        serde_json::from_slice(&text)
            .inspect_err(|error| tracing::warn!(%error, "Mirror config is unreadable; mirror off"))
            .ok()
    }

    /// When `path` is missing, moves a downloaded config from `downloads` into
    /// it: `mirror.json`, else the newest browser-renamed `mirror*.json`, the
    /// first of those that parses. It is copied then removed so no secret is
    /// left behind, and an existing `path` is never overwritten. A failed copy
    /// leaves no partial `path` and keeps the download. Values are never logged.
    pub fn adopt(path: &Path, downloads: &Path) {
        adopt_with(path, downloads, |file, bytes| file.write_all(bytes));
    }
}

/// [`Config::adopt`] with the write injectable, so a test can fail it.
fn adopt_with(
    path: &Path,
    downloads: &Path,
    write: impl FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
) {
    if path.exists() {
        return;
    }
    let mut candidates: Vec<(PathBuf, Option<std::time::SystemTime>)> = fs::read_dir(downloads)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("mirror") && name.ends_with(".json")
        })
        .map(|entry| {
            let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
            (entry.path(), modified)
        })
        .collect();
    // Plain `mirror.json` first, then the renamed ones newest first.
    candidates.sort_by_key(|(found, modified)| {
        (
            found.file_name().is_none_or(|name| name != "mirror.json"),
            std::cmp::Reverse(*modified),
        )
    });
    let Some((found, bytes)) = candidates.into_iter().find_map(|(found, _)| {
        let bytes = fs::read(&found).ok()?;
        let valid = serde_json::from_slice::<Config>(&bytes).is_ok_and(|config| {
            !config.bucket.is_empty()
                && !config.access_key.is_empty()
                && !config.secret_key.is_empty()
        });
        if !valid {
            // The parse error can quote a value, so it is not logged.
            tracing::warn!(file = %found.display(), "Downloaded mirror config is invalid (needs bucket, hmac_access_id, hmac_secret); left where it is");
            return None;
        }
        Some((found, bytes))
    }) else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            // It holds a credential: owner-only, whatever the umask.
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            options.open(path)
        })
        .and_then(|mut file| {
            let result = write(&mut file, &bytes);
            drop(file);
            if result.is_err() {
                // A partial file would pass the exists check and keep the mirror off for good.
                let _ = fs::remove_file(path);
            }
            result
        });
    if let Err(error) = written {
        tracing::warn!(%error, to = %path.display(), "Could not move the downloaded mirror config");
        return;
    }
    if let Err(error) = fs::remove_file(&found) {
        tracing::warn!(%error, file = %found.display(), "Mirror config copied, but the download could not be removed");
        return;
    }
    tracing::info!(from = %found.display(), to = %path.display(), "Moved the downloaded mirror config into app data");
}

#[derive(Debug)]
pub struct StopReport {
    pub frozen: bool,
    pub drained: bool,
    /// Why the take cannot complete; a deadline alone leaves this `None`.
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct TrackManifest {
    #[serde(default, alias = "fragments")]
    segments: Vec<Fragment>,
}

#[derive(Deserialize)]
struct Fragment {
    path: String,
    #[serde(default)]
    is_complete: bool,
    file_size: Option<u64>,
}

pub struct Mirror {
    config: Config,
    endpoint: String,
    take: PathBuf,
    outbox: PathBuf,
    prefix: String,
    client: Client,
    /// Object name under `prefix` -> SHA-256 of the accepted body.
    uploaded: HashMap<String, String>,
    /// Bytes of the frozen take in the bucket, counted as each PUT streams.
    sent: Arc<AtomicU64>,
    /// The highest `sent` published: a failed PUT's bytes drop out of `sent`,
    /// but the VM reads only a rise as progress, so the beat never goes lower.
    published: Arc<AtomicU64>,
    progress_every: Duration,
    stall: Duration,
    /// Made on first use, on the thread that uploads, never inside Cap's runtime.
    streaming: Option<(tokio::runtime::Runtime, reqwest::Client)>,
}

impl Mirror {
    /// `prefix` is the take's folder in the bucket; objects keep the take's
    /// relative paths under it.
    pub fn new(config: Config, take: &Path, outbox: &Path, prefix: &str) -> io::Result<Self> {
        fs::create_dir_all(outbox)?;
        let mut uploaded = HashMap::new();
        if let Ok(ledger) = fs::read_to_string(outbox.join(LEDGER)) {
            for line in ledger.lines() {
                if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line)
                    && let (Some(key), Some(sha)) =
                        (entry["key"].as_str(), entry["sha256"].as_str())
                {
                    uploaded.insert(key.to_string(), sha.to_string());
                }
            }
        }
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| "https://storage.googleapis.com".into());
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            config,
            take: take.to_path_buf(),
            outbox: outbox.to_path_buf(),
            prefix: prefix.trim_matches('/').to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(io::Error::other)?,
            uploaded,
            sent: Arc::new(AtomicU64::new(0)),
            published: Arc::new(AtomicU64::new(0)),
            progress_every: PROGRESS_EVERY,
            stall: STALL,
            streaming: None,
        })
    }

    pub fn set_endpoint(&mut self, endpoint: &str) {
        self.endpoint = endpoint.trim_end_matches('/').to_string();
    }

    pub fn set_progress_every(&mut self, every: Duration) {
        self.progress_every = every;
    }

    pub fn set_stall(&mut self, stall: Duration) {
        self.stall = stall;
    }

    fn signer(&self) -> sigv4::Key<'_> {
        sigv4::Key {
            access_key: &self.config.access_key,
            secret_key: &self.config.secret_key,
            region: &self.config.region,
            service: "s3",
        }
    }

    fn url(&self, key: &str) -> Result<Url, String> {
        let object = format!("{}/{}/{}", self.config.bucket, self.prefix, key).replace("//", "/");
        let path = percent_encoding::utf8_percent_encode(&object, sigv4::PATH);
        Url::parse(&format!("{}/{path}", self.endpoint)).map_err(|e| e.to_string())
    }

    fn send(
        &self,
        method: &str,
        key: &str,
        body: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        request(
            &self.client,
            &self.signer(),
            method,
            &self.url(key)?,
            body,
            timeout,
        )
    }

    /// PUTs one of the take's own files, counting its bytes into `sent` as the
    /// connection takes them. Fails after `timeout` in all, or once no bytes
    /// have moved for `stall` while the body still has bytes to give: a whole
    /// camera.mp4 can need many minutes. Its buffered tail and the response
    /// wait move no chunk, so only `timeout` bounds them.
    fn put_counted(&mut self, key: &str, body: Vec<u8>, timeout: Duration) -> Result<(), String> {
        let url = self.url(key)?;
        let headers = signed(&self.signer(), "PUT", &url, &body)?;
        let moved = Arc::new(Mutex::new(Some(Instant::now())));
        let length = body.len();
        let body = Counted {
            body: body.into(),
            sent: self.sent.clone(),
            moved: moved.clone(),
        };
        let stall = self.stall;
        let (runtime, client) = match &mut self.streaming {
            Some(streaming) => streaming,
            empty => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())?;
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .build()
                    .map_err(|e| e.to_string())?;
                empty.insert((runtime, client))
            }
        };
        let mut request = client
            .put(url.clone())
            .timeout(timeout)
            .header("content-length", length)
            .body(reqwest::Body::wrap(body));
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        runtime.block_on(async {
            let mut response = Box::pin(async {
                let response = request.send().await?;
                Ok::<_, reqwest::Error>((response.status(), response.bytes().await?))
            });
            loop {
                match tokio::time::timeout(stall.min(Duration::from_secs(1)), &mut response).await {
                    Ok(Ok((status, _))) if status.is_success() => return Ok(()),
                    Ok(Ok((status, _))) => return Err(format!("PUT {}: {status}", url.path())),
                    Ok(Err(error)) => return Err(error.to_string()),
                    Err(_)
                        if moved
                            .lock()
                            .unwrap()
                            .is_some_and(|at| at.elapsed() >= stall) =>
                    {
                        return Err(format!("PUT {}: no bytes moved for {stall:?}", url.path()));
                    }
                    Err(_) => {}
                }
            }
        })
    }

    /// PUTs `progress.json` every `progress_every` until the returned sender
    /// drops; the first one after `stop.json`, so it never leads the take.
    fn heartbeat(&self, total: u64) -> mpsc::Sender<()> {
        let (stop, stopped) = mpsc::channel::<()>();
        let (client, sent, every) = (self.client.clone(), self.sent.clone(), self.progress_every);
        let published = self.published.clone();
        let (config, url) = (self.config.clone(), self.url(PROGRESS));
        std::thread::spawn(move || {
            let Ok(url) = url else { return };
            let signer = sigv4::Key {
                access_key: &config.access_key,
                secret_key: &config.secret_key,
                region: &config.region,
                service: "s3",
            };
            while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(every) {
                let now = sent.load(Ordering::Relaxed);
                let body = json!({ "sent": published.fetch_max(now, Ordering::Relaxed).max(now), "total": total });
                if let Err(error) = request(
                    &client,
                    &signer,
                    "PUT",
                    &url,
                    body.to_string().into_bytes(),
                    every,
                ) {
                    tracing::warn!(%error, "Progress upload failed");
                }
            }
        });
        stop
    }

    pub fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        self.send("GET", key, Vec::new(), Duration::from_secs(30))
    }

    /// PUTs `body` unless the ledger already holds this exact object.
    /// Returns whether a request was made.
    pub fn put(&mut self, key: &str, body: Vec<u8>, timeout: Duration) -> Result<bool, String> {
        let sha = sigv4::sha256_hex(&body);
        self.put_hashed(key, body, sha, timeout)
    }

    fn put_hashed(
        &mut self,
        key: &str,
        body: Vec<u8>,
        sha: String,
        timeout: Duration,
    ) -> Result<bool, String> {
        if self.uploaded.get(key) == Some(&sha) {
            return Ok(false);
        }
        let size = body.len();
        if key == STOP || key == MANIFEST {
            self.send("PUT", key, body, timeout)?;
        } else {
            // only the take's own files count as progress
            self.put_counted(key, body, timeout)?;
        }
        let line = json!({ "key": key, "size": size, "sha256": sha }).to_string() + "\n";
        let mut ledger = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.outbox.join(LEDGER))
            .map_err(|e| e.to_string())?;
        ledger
            .write_all(line.as_bytes())
            .and_then(|()| ledger.sync_data())
            .map_err(|e| e.to_string())?;
        self.uploaded.insert(key.to_string(), sha);
        // Discarded while this PUT ran: take it back out.
        if discarded(&self.outbox) {
            self.delete(key)?;
            return Err("the take was discarded".into());
        }
        Ok(true)
    }

    fn segments(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = fs::read_dir(self.take.join("content/segments"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        dirs.sort();
        dirs
    }

    fn relative(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.take).unwrap_or(path);
        relative.to_string_lossy().replace('\\', "/")
    }

    /// One pass while recording, in D21's order: sidecars, audio deltas,
    /// screen, camera. Returns early once `stopping` is set.
    pub fn scan(&mut self, stopping: &AtomicBool) -> Result<(), String> {
        let timeout = Duration::from_secs(30);
        let segments = self.segments();
        let mut sidecars = vec![
            self.take.join("shots.json"),
            self.take.join("windows.jsonl"),
            self.take.join("recording-meta.json"),
            self.take.join("project-config.json"),
        ];
        for segment in &segments {
            sidecars.push(segment.join("start-times.json"));
            sidecars.push(segment.join("cursor.json"));
            sidecars.push(segment.join("keyboard.bin"));
        }
        for path in sidecars {
            if stopping.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Ok(body) = fs::read(&path) {
                self.put(&self.relative(&path), body, timeout)?;
            }
        }
        for segment in &segments {
            for track in m4a_files(segment) {
                let size = fs::metadata(&track).map_or(0, |m| m.len());
                let mut offset = 0;
                while offset + AUDIO_DELTA <= size && !stopping.load(Ordering::Relaxed) {
                    let key = format!("{}.part-{offset}", self.relative(&track));
                    if !self.uploaded.contains_key(&key) {
                        let mut file = fs::File::open(&track).map_err(|e| e.to_string())?;
                        let mut body = vec![0; AUDIO_DELTA as usize];
                        file.seek(SeekFrom::Start(offset))
                            .and_then(|_| file.read_exact(&mut body))
                            .map_err(|e| e.to_string())?;
                        self.put(&key, body, timeout)?;
                    }
                    offset += AUDIO_DELTA;
                }
            }
        }
        for name in ["display", "camera"] {
            for segment in &segments {
                let track = segment.join(name);
                let Ok(text) = fs::read(track.join("manifest.json")) else {
                    continue;
                };
                let Ok(manifest) = serde_json::from_slice::<TrackManifest>(&text) else {
                    continue;
                };
                let mut closed = vec![];
                for fragment in &manifest.segments {
                    let file = track.join(&fragment.path);
                    // A plain file name, complete, and exactly the size Cap recorded.
                    if fragment.is_complete
                        && Path::new(&fragment.path).file_name()
                            == Some(std::ffi::OsStr::new(&fragment.path))
                        && fs::metadata(&file).is_ok_and(|m| Some(m.len()) == fragment.file_size)
                    {
                        closed.push(file);
                    }
                }
                if !closed.is_empty() && track.join("init.mp4").is_file() {
                    closed.insert(0, track.join("init.mp4"));
                }
                for file in closed {
                    if stopping.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let relative = self.relative(&file);
                    if self.uploaded.contains_key(&relative) {
                        continue;
                    }
                    let staged = self.outbox.join(&relative);
                    link(&file, &staged).map_err(|e| format!("link {relative}: {e}"))?;
                    let body = fs::read(&staged).map_err(|e| e.to_string())?;
                    self.put(&relative, body, timeout)?;
                }
            }
        }
        Ok(())
    }

    /// Freezes the whole take into the outbox and writes `stop.json` listing
    /// every frozen file with its size and SHA-256. Local only, so Cap's
    /// remux may run once it returns. Any file it cannot stage or hash fails
    /// it: no `stop.json` is written, so no manifest can follow, and
    /// `mirror-failed.json` in the outbox says why. Cap's take is untouched.
    pub fn freeze(&mut self, stopped_at_unix_ms: u64) -> Result<(), String> {
        let frozen = self.stage(stopped_at_unix_ms);
        if let Err(error) = &frozen {
            tracing::error!(%error, take = %self.take.display(), "Mirror freeze failed; this take is not mirrored");
            let marker = json!({ "stopped_at_unix_ms": stopped_at_unix_ms, "error": error });
            let marker = serde_json::to_vec_pretty(&marker).expect("JSON value serializes");
            if let Err(error) = write_atomic(&self.outbox.join(FAILED), &marker) {
                tracing::error!(%error, "Could not write {FAILED}");
            }
        }
        frozen
    }

    fn stage(&mut self, stopped_at_unix_ms: u64) -> Result<(), String> {
        let mut files = vec![];
        walk(&self.take, &self.outbox, &mut files).map_err(|e| format!("list take: {e}"))?;
        let mut expected: Vec<String> = files.iter().map(|path| self.relative(path)).collect();
        expected.sort_by_key(|relative| (upload_rank(relative), relative.clone()));
        let mut listed = vec![];
        let mut errors = vec![];
        for relative in &expected {
            let (source, staged) = (self.take.join(relative), self.outbox.join(relative));
            let staged_ok = if is_media(relative) {
                link(&source, &staged)
            } else {
                fs::read(&source).and_then(|body| write_atomic(&staged, &body))
            };
            // Hashed as frozen, so the VM can trust a partial take (film A-9.4).
            match staged_ok.and_then(|()| hash_file(&staged)) {
                Ok((size, sha256)) => {
                    listed.push(json!({ "path": relative, "size": size, "sha256": sha256 }))
                }
                Err(error) => errors.push(format!("{relative}: {error}")),
            }
        }
        if !errors.is_empty() {
            return Err(format!("could not freeze {}", errors.join("; ")));
        }
        let stop = json!({
            "send_required": true,
            "stopped_at_unix_ms": stopped_at_unix_ms,
            "expected_files": listed,
        });
        let stop = serde_json::to_vec_pretty(&stop).expect("JSON value serializes");
        write_atomic(&self.outbox.join(STOP), &stop).map_err(|e| format!("write {STOP}: {e}"))
    }

    /// The stop sequence: the freeze, then the drain (which PUTs `stop.json`
    /// first) for at most `drain`. Whatever is left goes up later through
    /// [`Mirror::drain`].
    pub fn stop(&mut self, stopped_at_unix_ms: u64, drain: Duration) -> StopReport {
        if let Err(error) = self.freeze(stopped_at_unix_ms) {
            return StopReport {
                frozen: false,
                drained: false,
                error: Some(error),
            };
        }
        match self.drain(Some(Instant::now() + drain)) {
            Ok(drained) => StopReport {
                frozen: true,
                drained,
                error: None,
            },
            Err(error) => StopReport {
                frozen: true,
                drained: false,
                error: Some(error),
            },
        }
    }

    /// The presenter's Send: PUTs `send.json`, which lets the film watcher
    /// build the take once its manifest lands. A retry re-sends the first
    /// decision unchanged; a discarded take is never sent.
    pub fn send_to_film(&mut self) -> Result<(), String> {
        if discarded(&self.outbox) {
            return Err("the take was discarded".into());
        }
        let path = self.outbox.join(SEND);
        let body = match fs::read(&path) {
            Ok(body) => body,
            Err(_) => {
                let choice = json!({
                    "send": true,
                    "decided_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    "cap_send_version": CAP_SEND_VERSION,
                });
                let body = serde_json::to_vec_pretty(&choice).expect("JSON value serializes");
                write_atomic(&path, &body).map_err(|e| format!("write {SEND}: {e}"))?;
                body
            }
        };
        self.put(SEND, body, Duration::from_secs(30)).map(|_| ())
    }

    /// The presenter's Discard: marks the take so nothing more of it goes up,
    /// then deletes every object of it in the bucket. A sent take stays sent.
    pub fn discard(&mut self) -> Result<(), String> {
        if self.outbox.join(SEND).is_file() {
            return Err("the take is already sent to film".into());
        }
        write_atomic(&self.outbox.join(DISCARDED), b"{}")
            .map_err(|e| format!("write {DISCARDED}: {e}"))?;
        // Read after the mark: a PUT the drain finishes later deletes itself (put_hashed).
        let mut keys: Vec<String> = fs::read_to_string(self.outbox.join(LEDGER))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|entry| entry["key"].as_str().map(str::to_string))
            .chain([PROGRESS.to_string()])
            .collect();
        keys.sort();
        keys.dedup();
        let failed: Vec<String> = keys
            .iter()
            .filter_map(|key| self.delete(key).err())
            .collect();
        if failed.is_empty() {
            Ok(())
        } else {
            // shortcut: a failed delete is not retried; the objects left never
            // get a send.json, so the watcher never builds them.
            Err(format!(
                "{} objects stay in the bucket: {}",
                failed.len(),
                failed.join("; ")
            ))
        }
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match self.send("DELETE", key, Vec::new(), Duration::from_secs(30)) {
            Err(error) if !error.ends_with("404 Not Found") => Err(error),
            _ => Ok(()),
        }
    }

    /// Uploads what the outbox still holds after a stop, then the manifest.
    /// `Ok(true)`: the take is complete in the bucket. `Ok(false)`: the
    /// deadline passed first; call again. `Err`: the take was discarded, or
    /// the frozen take does not match `stop.json` (missing, changed, or a
    /// path outside the outbox), so the manifest is never written.
    pub fn drain(&mut self, deadline: Option<Instant>) -> Result<bool, String> {
        if discarded(&self.outbox) {
            return Err("the take was discarded".into());
        }
        if self.uploaded.contains_key(MANIFEST) {
            return Ok(true);
        }
        let stop = fs::read(self.outbox.join(STOP)).map_err(|e| format!("read {STOP}: {e}"))?;
        let expected = serde_json::from_slice::<StopFile>(&stop)
            .map_err(|e| format!("parse {STOP}: {e}"))?
            .expected_files;
        if let Some(bad) = expected.iter().find(|file| !inside(&file.path)) {
            return Err(format!(
                "{STOP} lists a path outside the take: {:?}",
                bad.path
            ));
        }
        let outbox = fs::canonicalize(&self.outbox).map_err(|e| format!("outbox: {e}"))?;
        let stop_sha = sigv4::sha256_hex(&stop);
        self.sent.store(0, Ordering::Relaxed);
        let _beating = self.heartbeat(expected.iter().map(|file| file.size).sum());
        let mut backoff = Duration::from_secs(1);
        loop {
            if discarded(&self.outbox) {
                return Err("the take was discarded".into());
            }
            let mut failed = false;
            // Counted up front, so a retry pass never reports fewer bytes than the last.
            let mut done: u64 = expected
                .iter()
                .filter(|file| self.uploaded.get(&file.path) == Some(&file.sha256))
                .map(|file| file.size)
                .sum();
            let timeout = deadline.map_or(Duration::from_secs(120), remaining);
            if let Err(error) = self.put_hashed(STOP, stop.clone(), stop_sha.clone(), timeout) {
                tracing::warn!(%error, "Upload failed; retrying");
                failed = true;
            }
            for file in &expected {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Ok(false);
                }
                if discarded(&self.outbox) {
                    return Err("the take was discarded".into());
                }
                // The bucket already holds exactly this object.
                if self.uploaded.get(&file.path) == Some(&file.sha256) {
                    continue;
                }
                let body = read_frozen(&outbox, file)?;
                // Not cut at the deadline: a file cut part-way restarts from
                // its first byte. The deadline only stops the next one starting.
                let timeout = Duration::from_secs((file.size / BYTES_PER_SECOND_FLOOR).max(120));
                // A failed PUT's streamed bytes do not count.
                self.sent.store(done, Ordering::Relaxed);
                match self.put_hashed(&file.path, body, file.sha256.clone(), timeout) {
                    Ok(_) => done += file.size,
                    Err(error) => {
                        tracing::warn!(%error, "Upload failed; retrying");
                        failed = true;
                    }
                }
                self.sent.store(done, Ordering::Relaxed);
            }
            if !failed {
                let manifest = json!({
                    "files": expected,
                    "fork_commit": FORK_COMMIT,
                    "contracts_version": CONTRACTS_VERSION,
                    "project_format_version": PROJECT_FORMAT_VERSION,
                });
                let manifest = serde_json::to_vec_pretty(&manifest).expect("JSON value serializes");
                let _ = write_atomic(&self.outbox.join(MANIFEST), &manifest);
                let timeout = deadline.map_or(Duration::from_secs(120), remaining);
                match self.put(MANIFEST, manifest, timeout) {
                    Ok(_) => return Ok(true),
                    Err(error) => tracing::warn!(%error, "Manifest upload failed; retrying"),
                }
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(false);
            }
            let wait = deadline.map_or(backoff, |deadline| backoff.min(remaining(deadline)));
            std::thread::sleep(wait);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    /// Runs the mirror on its own thread: a scan every `every` until
    /// [`Worker::stop`] (or the take folder disappears).
    pub fn spawn(mut self, every: Duration) -> io::Result<Worker> {
        let stopping = Arc::new(AtomicBool::new(false));
        let (stop, stops) = mpsc::channel::<StopRequest>();
        let scan_stopping = stopping.clone();
        std::thread::Builder::new()
            .name("gcs-mirror".into())
            .spawn(move || {
                loop {
                    match stops.recv_timeout(every) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            // A deleted take has nothing left to mirror.
                            if !self.take.is_dir() {
                                return;
                            }
                            if let Err(error) = self.scan(&scan_stopping) {
                                tracing::debug!(%error, "Mirror scan incomplete; next pass retries");
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Ok(request) => return self.stop_requested(request),
                    }
                }
            })?;
        Ok(Worker { stopping, stop })
    }

    fn stop_requested(mut self, request: StopRequest) {
        let frozen = self.freeze(request.stopped_at_unix_ms);
        let failed = frozen.is_err();
        let _ = request.frozen.send(frozen);
        if failed {
            return;
        }
        // While the presenter decides; send.json goes up on its own (send_to_film).
        if let Err(error) = self.drain(None) {
            tracing::error!(%error, "Mirror cannot complete this take");
        }
    }
}

struct StopRequest {
    stopped_at_unix_ms: u64,
    frozen: mpsc::Sender<Result<(), String>>,
}

/// The handle to a mirror running on its own thread.
pub struct Worker {
    stopping: Arc<AtomicBool>,
    stop: mpsc::Sender<StopRequest>,
}

impl Worker {
    /// Runs the stop sequence. Blocks, with no time limit, until the freeze
    /// is over: `Err` means this take is not mirrored (`mirror-failed.json`
    /// says why), and Cap carries on as without a mirror. The drain then runs
    /// on in the background while the presenter chooses Send or Discard.
    pub fn stop(self, stopped_at_unix_ms: u64) -> Result<(), String> {
        self.stopping.store(true, Ordering::Relaxed);
        let (frozen, frozen_rx) = mpsc::channel();
        self.stop
            .send(StopRequest {
                stopped_at_unix_ms,
                frozen,
            })
            .map_err(|_| "mirror thread has exited".to_string())?;
        frozen_rx
            .recv()
            .map_err(|_| "mirror thread exited before the freeze".to_string())?
    }
}

#[derive(Deserialize, serde::Serialize)]
struct Expected {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct StopFile {
    expected_files: Vec<Expected>,
}

/// A plain relative path inside the take that is none of the mirror's own files.
fn inside(relative: &str) -> bool {
    let path = Path::new(relative);
    ![STOP, MANIFEST, LEDGER, FAILED, PROGRESS, SEND, DISCARDED].contains(&relative)
        && path.components().next().is_some()
        && path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

/// Reads a frozen file only if it is a regular file inside the outbox and
/// exactly the size and SHA-256 `stop.json` recorded for it.
fn read_frozen(outbox: &Path, file: &Expected) -> Result<Vec<u8>, String> {
    let staged = outbox.join(&file.path);
    let not_frozen = |e: io::Error| format!("frozen {} unreadable: {e}", file.path);
    let real = fs::canonicalize(&staged).map_err(not_frozen)?;
    if !real.starts_with(outbox) || !fs::symlink_metadata(&staged).map_err(not_frozen)?.is_file() {
        return Err(format!("frozen {} resolves outside the outbox", file.path));
    }
    let body = fs::read(&real).map_err(not_frozen)?;
    if body.len() as u64 != file.size || sigv4::sha256_hex(&body) != file.sha256 {
        return Err(format!("frozen {} no longer matches {STOP}", file.path));
    }
    Ok(body)
}

/// One SigV4-signed XML-API request to any object URL; the body on success.
pub fn request(
    client: &Client,
    signer: &sigv4::Key,
    method: &str,
    url: &Url,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let headers = signed(signer, method, url, &body)?;
    let mut request = client
        .request(method.parse().map_err(|_| "bad method")?, url.clone())
        .timeout(timeout)
        .body(body);
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    let response = request.send().map_err(|e| e.to_string())?;
    let status = response.status();
    let bytes = response.bytes().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{method} {}: {status}", url.path()));
    }
    Ok(bytes.to_vec())
}

/// The SigV4 headers for one request.
fn signed(
    signer: &sigv4::Key,
    method: &str,
    url: &Url,
    body: &[u8],
) -> Result<BTreeMap<String, String>, String> {
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    };
    let sha = sigv4::sha256_hex(body);
    let mut headers = BTreeMap::from([
        ("host".to_string(), host),
        ("x-amz-content-sha256".to_string(), sha.clone()),
        (
            "x-amz-date".to_string(),
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
        ),
    ]);
    let (authorization, _) = sigv4::authorize(signer, method, url, &headers, &sha)?;
    headers.insert("authorization".into(), authorization);
    Ok(headers)
}

/// A request body that adds each chunk to `sent`, and notes when, as the
/// connection takes it: a stalled connection stops taking chunks.
struct Counted {
    body: bytes::Bytes,
    sent: Arc<AtomicU64>,
    /// `None` once the last chunk is taken: no stall applies after that.
    moved: Arc<Mutex<Option<Instant>>>,
}

impl http_body::Body for Counted {
    type Data = bytes::Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<bytes::Bytes>, Self::Error>>> {
        if self.body.is_empty() {
            return Poll::Ready(None);
        }
        let take = self.body.len().min(64 * 1024);
        let chunk = self.body.split_to(take);
        self.sent.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        *self.moved.lock().unwrap() = (!self.body.is_empty()).then(Instant::now);
        Poll::Ready(Some(Ok(http_body::Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_empty()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.body.len() as u64)
    }
}

fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut hasher = sha2::Sha256::default();
    let size = io::copy(&mut fs::File::open(path)?, &mut hasher)?;
    Ok((size, hex::encode(sha2::Digest::finalize(hasher))))
}

fn remaining(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1))
}

/// D21's upload order: sidecars, then audio, then screen, then camera.
/// The prefix match also ranks the unfragmented `display.mp4` and
/// `camera.mp4` (macOS with a camera) behind the sidecars.
fn upload_rank(relative: &str) -> u8 {
    if relative.contains("/display") {
        2
    } else if relative.contains("/camera") {
        3
    } else if relative.ends_with(".m4a") {
        1
    } else {
        0
    }
}

/// Fragments and init segments: Cap never rewrites them, so a hard link is a
/// faithful, free copy that survives the remux deleting the original.
fn is_media(relative: &str) -> bool {
    (relative.contains("/display/") || relative.contains("/camera/"))
        && !relative.ends_with(".json")
}

fn m4a_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "m4a"))
        .collect();
    files.sort();
    files
}

fn walk(dir: &Path, skip: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path == skip {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&path, skip, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// Hard-links `source` to `staged`, falling back to a copy (another volume).
/// An existing `staged` is already the linked fragment and is kept.
fn link(source: &Path, staged: &Path) -> io::Result<()> {
    if staged.exists() {
        return Ok(());
    }
    fs::create_dir_all(staged.parent().expect("staged files sit in the outbox"))?;
    fs::hard_link(source, staged)
        .or_else(|_| fs::read(source).and_then(|body| write_atomic(staged, &body)))
}

/// Writes a new file and renames it into place, so a staged hard link is
/// replaced rather than written through to the file Cap owns.
fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    fs::create_dir_all(path.parent().expect("outbox files have a parent"))?;
    let temporary = path.with_extension("mirror-tmp");
    fs::write(&temporary, body)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_write_leaves_no_partial_config_and_keeps_the_download() {
        let dir =
            std::env::temp_dir().join(format!("gcs-mirror-adopt-partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let valid = r#"{"bucket":"b","hmac_access_id":"a","hmac_secret":"s"}"#;
        fs::write(dir.join("mirror.json"), valid).unwrap();
        let target = dir.join("appdata/mirror.json");

        adopt_with(&target, &dir, |file, bytes| {
            file.write_all(&bytes[..5])?;
            Err(io::Error::other("disk full"))
        });

        assert!(!target.exists(), "a partial config was left in app data");
        assert_eq!(fs::read_to_string(dir.join("mirror.json")).unwrap(), valid);
    }
}
