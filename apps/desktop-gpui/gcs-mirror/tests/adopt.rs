//! A `mirror.json` left in Downloads moves into app data only when it is
//! valid and app data has none.

use cap_gcs_mirror::Config;
use std::{fs, path::PathBuf};

const VALID: &str = r#"{"bucket":"b","hmac_access_id":"a","hmac_secret":"s","region":"auto"}"#;

/// A fresh `downloads/` and an empty `appdata/` beside it.
fn dirs(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("gcs-mirror-adopt-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("downloads")).unwrap();
    (dir.join("downloads"), dir.join("appdata"))
}

#[test]
fn a_valid_download_moves_into_app_data() {
    let (downloads, appdata) = dirs("moves");
    fs::write(downloads.join("mirror.json"), VALID).unwrap();
    let target = appdata.join("mirror.json");

    Config::adopt(&target, &downloads);

    assert_eq!(fs::read_to_string(&target).unwrap(), VALID);
    assert!(
        !downloads.join("mirror.json").exists(),
        "the secret stayed in Downloads"
    );
    assert_eq!(Config::load(&target).unwrap().bucket, "b");
}

#[test]
fn a_browser_renamed_download_moves_too() {
    let (downloads, appdata) = dirs("renamed");
    fs::write(downloads.join("mirror (1).json"), VALID).unwrap();
    let target = appdata.join("mirror.json");

    Config::adopt(&target, &downloads);

    assert_eq!(fs::read_to_string(&target).unwrap(), VALID);
    assert!(!downloads.join("mirror (1).json").exists());
}

#[test]
fn an_invalid_download_stays_where_it_is() {
    let (downloads, appdata) = dirs("invalid");
    let partial = r#"{"bucket":"b","region":"auto"}"#;
    fs::write(downloads.join("mirror.json"), partial).unwrap();
    let target = appdata.join("mirror.json");

    Config::adopt(&target, &downloads);

    assert!(!target.exists());
    assert_eq!(
        fs::read_to_string(downloads.join("mirror.json")).unwrap(),
        partial
    );
}

#[test]
fn an_existing_app_data_config_leaves_downloads_untouched() {
    let (downloads, appdata) = dirs("existing");
    fs::create_dir_all(&appdata).unwrap();
    let existing = r#"{"bucket":"kept","hmac_access_id":"a","hmac_secret":"s"}"#;
    fs::write(appdata.join("mirror.json"), existing).unwrap();
    fs::write(downloads.join("mirror.json"), VALID).unwrap();

    Config::adopt(&appdata.join("mirror.json"), &downloads);

    assert_eq!(
        fs::read_to_string(appdata.join("mirror.json")).unwrap(),
        existing
    );
    assert_eq!(
        fs::read_to_string(downloads.join("mirror.json")).unwrap(),
        VALID
    );
}

#[test]
fn an_invalid_plain_download_does_not_mask_a_valid_renamed_one() {
    let (downloads, appdata) = dirs("masked");
    let partial = r#"{"bucket":"b"}"#;
    fs::write(downloads.join("mirror.json"), partial).unwrap();
    fs::write(downloads.join("mirror (1).json"), VALID).unwrap();
    let target = appdata.join("mirror.json");

    Config::adopt(&target, &downloads);

    assert_eq!(fs::read_to_string(&target).unwrap(), VALID);
    assert!(!downloads.join("mirror (1).json").exists());
    assert_eq!(
        fs::read_to_string(downloads.join("mirror.json")).unwrap(),
        partial
    );
}

#[test]
fn the_download_stays_when_app_data_cannot_be_written() {
    let (downloads, appdata) = dirs("unwritable");
    // App data's parent is a file, so the folder cannot be made.
    fs::write(&appdata, "not a folder").unwrap();
    fs::write(downloads.join("mirror.json"), VALID).unwrap();

    Config::adopt(&appdata.join("mirror.json"), &downloads);

    assert_eq!(
        fs::read_to_string(downloads.join("mirror.json")).unwrap(),
        VALID
    );
}

#[cfg(unix)]
#[test]
fn the_adopted_config_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let (downloads, appdata) = dirs("mode");
    fs::write(downloads.join("mirror.json"), VALID).unwrap();
    let target = appdata.join("mirror.json");

    Config::adopt(&target, &downloads);

    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode {mode:o}");
}
