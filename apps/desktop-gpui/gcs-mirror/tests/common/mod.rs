//! A take folder, a mirror pointed at a fake bucket, and that bucket.
#![allow(dead_code)] // Each test file uses a different part.

use cap_gcs_mirror::{Config, Mirror};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Accepts every request with 200 and records `(method, path)`.
pub fn bucket() -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(vec![]));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or_default(),
                parts.next().unwrap_or_default(),
            );
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            log.lock()
                .unwrap()
                .push((method.to_string(), path.to_string()));
            reader
                .into_inner()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        }
    });
    (endpoint, seen)
}

/// A fresh folder holding `take/` and `outbox/`.
pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gcs-mirror-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("take/content/segments/segment-0")).unwrap();
    fs::create_dir_all(dir.join("outbox")).unwrap();
    dir
}

pub fn mirror(dir: &PathBuf, endpoint: &str) -> Mirror {
    let config = dir.join("config.json");
    fs::write(
        &config,
        r#"{"bucket":"b","access_key":"a","secret_key":"s","region":"auto"}"#,
    )
    .unwrap();
    let mut mirror = Mirror::new(
        Config::load(&config).unwrap(),
        &dir.join("take"),
        &dir.join("outbox"),
        "takes/t",
    )
    .unwrap();
    mirror.set_endpoint(endpoint);
    mirror
}

pub fn manifest_put(seen: &Mutex<Vec<(String, String)>>) -> bool {
    seen.lock()
        .unwrap()
        .iter()
        .any(|(method, path)| method == "PUT" && path.ends_with("/take-manifest.json"))
}
