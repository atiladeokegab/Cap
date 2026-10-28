//! Take metadata reaches the bucket while recording, and ahead of the
//! screen and camera at stop.

mod common;

use common::{bucket, folder, mirror};
use std::{fs, sync::atomic::AtomicBool, time::Duration};

#[test]
fn the_live_pass_uploads_the_take_metadata() {
    let (endpoint, seen) = bucket();
    let dir = folder("sidecars-live");
    let take = dir.join("take");
    fs::write(take.join("recording-meta.json"), b"{}").unwrap();
    fs::write(take.join("project-config.json"), b"{}").unwrap();
    fs::write(
        take.join("content/segments/segment-0/start-times.json"),
        b"{}",
    )
    .unwrap();
    mirror(&dir, &endpoint)
        .scan(&AtomicBool::new(false))
        .unwrap();
    let puts: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(_, path)| path.clone())
        .collect();
    for key in [
        "recording-meta.json",
        "project-config.json",
        "content/segments/segment-0/start-times.json",
    ] {
        assert!(
            puts.contains(&format!("/b/takes/t/{key}")),
            "{key} not in {puts:?}"
        );
    }
}

#[test]
fn an_unfragmented_screen_goes_up_after_the_metadata() {
    let (endpoint, seen) = bucket();
    let dir = folder("sidecars-order");
    let take = dir.join("take");
    // macOS with a camera records whole MP4s, which sort before
    // `recording-meta.json` by name alone. No live pass: only the stop order.
    fs::write(
        take.join("content/segments/segment-0/camera.mp4"),
        b"camera",
    )
    .unwrap();
    fs::write(
        take.join("content/segments/segment-0/display.mp4"),
        b"screen",
    )
    .unwrap();
    fs::write(take.join("recording-meta.json"), b"{}").unwrap();
    let report = mirror(&dir, &endpoint).stop(1, Duration::from_secs(5));
    assert!(report.drained, "{:?}", report.error);
    let puts: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(_, path)| path.clone())
        .collect();
    let at = |key: &str| puts.iter().position(|path| path.ends_with(key)).unwrap();
    assert!(at("recording-meta.json") < at("display.mp4"), "{puts:?}");
    assert!(at("display.mp4") < at("camera.mp4"), "{puts:?}");
}
