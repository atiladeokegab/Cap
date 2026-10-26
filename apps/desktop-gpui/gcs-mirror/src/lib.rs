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
    sync::atomic::{AtomicBool, Ordering},
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
}

#[derive(Debug)]
pub struct StopReport {
    pub frozen: bool,
    pub drained: bool,
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
        })
    }

    pub fn set_endpoint(&mut self, endpoint: &str) {
        self.endpoint = endpoint.trim_end_matches('/').to_string();
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
        let signer = sigv4::Key {
            access_key: &self.config.access_key,
            secret_key: &self.config.secret_key,
            region: &self.config.region,
            service: "s3",
        };
        request(
            &self.client,
            &signer,
            method,
            &self.url(key)?,
            body,
            timeout,
        )
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
        self.send("PUT", key, body, timeout)?;
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
        ];
        for segment in &segments {
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

    /// The stop sequence: `stop.json` first, then the whole take frozen into
    /// the outbox, then the drain for at most `drain`. Call it before Cap's
    /// remux; whatever is left goes up later through [`Mirror::drain`].
    pub fn stop(&mut self, stopped_at_unix_ms: u64, drain: Duration) -> StopReport {
        let deadline = Instant::now() + drain;
        let mut files = vec![];
        walk(&self.take, &self.outbox, &mut files);
        let mut expected: Vec<String> = files.iter().map(|path| self.relative(path)).collect();
        expected.sort_by_key(|relative| (upload_rank(relative), relative.clone()));
        let stop = json!({ "stopped_at_unix_ms": stopped_at_unix_ms, "expected_files": expected });
        let stop = serde_json::to_vec_pretty(&stop).expect("JSON value serializes");
        let mut frozen = write_atomic(&self.outbox.join(STOP), &stop).is_ok();
        if let Err(error) = self.put(STOP, stop, remaining(deadline).min(Duration::from_secs(15))) {
            tracing::warn!(%error, "stop.json not uploaded yet");
        }
        for relative in &expected {
            let (source, staged) = (self.take.join(relative), self.outbox.join(relative));
            let result = if is_media(relative) {
                link(&source, &staged)
            } else {
                fs::read(&source).and_then(|body| write_atomic(&staged, &body))
            };
            if let Err(error) = result {
                tracing::warn!(%error, relative, "Could not freeze a take file");
                frozen = false;
            }
        }
        let drained = self.drain(Some(deadline));
        StopReport { frozen, drained }
    }

    /// Uploads what the outbox still holds after a stop, then the manifest.
    /// Returns whether the take is complete in the bucket.
    pub fn drain(&mut self, deadline: Option<Instant>) -> bool {
        if self.uploaded.contains_key(MANIFEST) {
            return true;
        }
        let Ok(stop) = fs::read(self.outbox.join(STOP)) else {
            return false;
        };
        let Ok(stop) = serde_json::from_slice::<serde_json::Value>(&stop) else {
            return false;
        };
        let expected: Vec<String> = stop["expected_files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|path| path.as_str().map(String::from))
            .collect();
        let mut backoff = Duration::from_secs(1);
        loop {
            let mut files = vec![];
            let mut failed = false;
            for key in std::iter::once(STOP.to_string()).chain(expected.iter().cloned()) {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return false;
                }
                let Ok(body) = fs::read(self.outbox.join(&key)) else {
                    tracing::warn!(key, "Missing from the outbox; left out of the manifest");
                    continue;
                };
                let sha = sigv4::sha256_hex(&body);
                if key != STOP {
                    files.push(json!({ "path": key, "size": body.len(), "sha256": sha }));
                }
                let timeout = deadline.map_or(Duration::from_secs(120), remaining);
                if let Err(error) = self.put_hashed(&key, body, sha, timeout) {
                    tracing::warn!(%error, "Upload failed; retrying");
                    failed = true;
                }
            }
            if !failed {
                let manifest = json!({
                    "files": files,
                    "fork_commit": FORK_COMMIT,
                    "contracts_version": CONTRACTS_VERSION,
                    "project_format_version": PROJECT_FORMAT_VERSION,
                });
                let manifest = serde_json::to_vec_pretty(&manifest).expect("JSON value serializes");
                let _ = write_atomic(&self.outbox.join(MANIFEST), &manifest);
                let timeout = deadline.map_or(Duration::from_secs(120), remaining);
                match self.put(MANIFEST, manifest, timeout) {
                    Ok(_) => return true,
                    Err(error) => tracing::warn!(%error, "Manifest upload failed; retrying"),
                }
            }
            let wait = deadline.map_or(backoff, |deadline| backoff.min(remaining(deadline)));
            std::thread::sleep(wait);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
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
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    };
    let sha = sigv4::sha256_hex(&body);
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

fn remaining(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1))
}

/// D21's upload order: sidecars, then audio, then screen, then camera.
fn upload_rank(relative: &str) -> u8 {
    if relative.contains("/display/") {
        2
    } else if relative.contains("/camera/") {
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
    upload_rank(relative) >= 2 && !relative.ends_with(".json")
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

fn walk(dir: &Path, skip: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path == skip {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, skip, files),
            Ok(kind) if kind.is_file() => files.push(path),
            _ => {}
        }
    }
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
