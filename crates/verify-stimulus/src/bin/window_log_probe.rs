//! Real-Windows probe for the fork's window log: `apps/desktop-gpui/src/window_log.rs`, compiled in
//! unchanged, polls the foreground window into `<dir>/windows.jsonl` while the fixture records, until
//! `<stop>` exists. The fixture's take is recorded by cap-verify, not the app, so these times are on the
//! probe's own clock, whose start is `<dir>/window-log-epoch.json` in unix ms, not the take's start gate.
//!
//!     window-log-probe <dir> <stop>

#[allow(dead_code)]
#[path = "../../../../apps/desktop-gpui/src/window_log.rs"]
mod window_log;

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (Some(dir), Some(stop)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: window-log-probe <dir> <stop>");
    };
    std::fs::create_dir_all(&dir)?;
    let start = Instant::now();
    let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    std::fs::write(
        dir.join("window-log-epoch.json"),
        serde_json::json!({ "epoch_unix_ms": unix_ms }).to_string(),
    )?;
    let _log = window_log::spawn(&dir, 0, move || Some(start));
    let deadline = start + Duration::from_secs(30 * 60);
    while !stop.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}
