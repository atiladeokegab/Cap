use std::path::{Path, PathBuf};

use cap_project::{
    InstantRecordingMeta, RecordingMeta, RecordingMetaInner, StudioRecordingMeta,
    StudioRecordingStatus,
};
use cap_rendering::ProjectRecordingsMeta;
use serde::Serialize;

use crate::{OutputFormat, write_json};

pub fn schema() -> Result<(), String> {
    write_json(&cap_project::ProjectConfiguration::json_schema())
}

fn parse_config(settings_json: &str) -> Result<cap_project::ProjectConfiguration, String> {
    let document: serde_json::Value = serde_json::from_str(settings_json)
        .map_err(|e| format!("Invalid project config JSON: {e}"))?;
    let schema = cap_project::ProjectConfiguration::json_schema();
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| format!("Invalid project config schema: {e}"))?;
    if let Err(error) = validator.validate(&document) {
        return Err(format!(
            "Invalid project config at {}: {error}",
            config_error_path(&error)
        ));
    }
    serde_json::from_value(document).map_err(|e| format!("Invalid project config JSON: {e}"))
}

fn config_error_path(error: &jsonschema::ValidationError<'_>) -> String {
    use jsonschema::error::ValidationErrorKind;
    match error.kind() {
        ValidationErrorKind::AnyOf { context } | ValidationErrorKind::OneOfNotValid { context } => {
            context
                .iter()
                .flatten()
                .map(config_error_path)
                .collect::<Vec<_>>()
                .join(", ")
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => unexpected
            .iter()
            .map(|field| {
                let field = field.replace('~', "~0").replace('/', "~1");
                format!("{}/{field}", error.instance_path())
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => error.instance_path().to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInspection {
    pub project_path: PathBuf,
    pub output_path: PathBuf,
    /// camelCase convenience fields so agents never reach into the snake_case `meta` passthrough.
    pub name: String,
    pub recording_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    pub meta: RecordingMeta,
    pub config: cap_project::ProjectConfiguration,
}

pub fn config_get(project_path: PathBuf) -> Result<(), String> {
    RecordingMeta::load_for_project(&project_path)
        .map_err(|e| format!("Failed to load recording meta: {e}"))?;
    let config = match cap_project::ProjectConfiguration::load(&project_path) {
        Ok(config) => config,
        // Instant and un-edited studio recordings have no project-config.json; return the
        // effective default the editor/exporter would use rather than erroring.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(format!("Failed to load project config: {e}")),
    };
    crate::write_json(&config)
}

pub fn config_set(
    project_path: PathBuf,
    settings_json: &str,
    format: OutputFormat,
) -> Result<(), String> {
    let config = parse_config(settings_json)?;
    RecordingMeta::load_for_project(&project_path)
        .map_err(|e| format!("Failed to load recording meta: {e}"))?;
    // write() validates internally before its atomic temp-file-then-rename.
    config
        .write(&project_path)
        .map_err(|e| format!("Failed to write project config: {e}"))?;
    if let OutputFormat::Json = format {
        crate::write_json(&serde_json::json!({ "ok": true }))?;
    }
    Ok(())
}

pub fn inspect(project_path: PathBuf, format: OutputFormat) -> Result<(), String> {
    let meta = RecordingMeta::load_for_project(&project_path)
        .map_err(|e| format!("Failed to load recording meta: {e}"))?;
    let output_path = meta.output_path();
    let config = meta.project_config();
    let duration_seconds = project_duration_seconds(&project_path, &meta);

    match format {
        OutputFormat::Text => {
            println!("project: {}", project_path.display());
            println!("name: {}", meta.pretty_name);
            println!("type: {}", recording_type(&meta));
            println!("output: {}", output_path.display());
            if let Some(duration) = duration_seconds {
                println!("duration: {duration:.2}s");
            }
            Ok(())
        }
        OutputFormat::Json => write_json(&ProjectInspection {
            project_path,
            output_path,
            name: meta.pretty_name.clone(),
            recording_type: recording_type(&meta),
            duration_seconds,
            meta,
            config,
        }),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileCheck {
    role: &'static str,
    path: PathBuf,
    exists: bool,
    required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationReport {
    project_path: PathBuf,
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recording_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    checks: Vec<FileCheck>,
    missing: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    problems: Vec<String>,
}

fn recording_type(meta: &RecordingMeta) -> &'static str {
    match meta.inner {
        RecordingMetaInner::Studio(_) => "studio",
        RecordingMetaInner::Instant(_) => "instant",
    }
}

fn project_duration_seconds(project_path: &Path, meta: &RecordingMeta) -> Option<f64> {
    let RecordingMetaInner::Studio(studio) = &meta.inner else {
        return None;
    };

    let recordings = ProjectRecordingsMeta::new(&project_path.to_path_buf(), studio).ok()?;

    Some(cap_rendering::get_duration(
        &recordings,
        meta,
        studio,
        &meta.project_config(),
    ))
}

fn required_check(role: &'static str, path: PathBuf) -> FileCheck {
    FileCheck {
        exists: path.exists(),
        role,
        path,
        required: true,
    }
}

fn optional_check(role: &'static str, path: PathBuf) -> FileCheck {
    FileCheck {
        exists: path.exists(),
        role,
        path,
        required: false,
    }
}

fn studio_checks(meta: &RecordingMeta, studio: &StudioRecordingMeta) -> Vec<FileCheck> {
    let mut checks = Vec::new();

    match studio {
        StudioRecordingMeta::SingleSegment { segment } => {
            checks.push(required_check(
                "displayVideo",
                meta.path(&segment.display.path),
            ));
            if let Some(camera) = &segment.camera {
                checks.push(required_check("camera", meta.path(&camera.path)));
            }
            if let Some(audio) = &segment.audio {
                checks.push(required_check("audio", meta.path(&audio.path)));
            }
            if let Some(cursor) = &segment.cursor {
                checks.push(optional_check("cursor", meta.path(cursor)));
            }
        }
        StudioRecordingMeta::MultipleSegments { inner } => {
            for segment in &inner.segments {
                checks.push(required_check(
                    "displayVideo",
                    meta.path(&segment.display.path),
                ));
                if let Some(camera) = &segment.camera {
                    checks.push(required_check("camera", meta.path(&camera.path)));
                }
                if let Some(mic) = &segment.mic {
                    checks.push(required_check("mic", meta.path(&mic.path)));
                }
                if let Some(system_audio) = &segment.system_audio {
                    checks.push(required_check("systemAudio", meta.path(&system_audio.path)));
                }
                if let Some(cursor) = &segment.cursor {
                    checks.push(optional_check("cursor", meta.path(cursor)));
                }
            }
        }
    }

    checks
}

fn studio_problems(studio: &StudioRecordingMeta) -> Vec<String> {
    let mut problems = Vec::new();

    if let StudioRecordingMeta::MultipleSegments { inner } = studio {
        if inner.segments.is_empty() {
            problems.push("studio recording has no segments".to_string());
        }
    }

    match studio.status() {
        StudioRecordingStatus::Complete => {}
        StudioRecordingStatus::InProgress => {
            problems.push("studio recording is still in progress".to_string());
        }
        StudioRecordingStatus::NeedsRemux => {
            problems.push("studio recording still needs remux".to_string());
        }
        StudioRecordingStatus::Failed { error } => {
            problems.push(format!("studio recording failed: {error}"));
        }
    }

    problems
}

fn instant_problems(instant: &InstantRecordingMeta) -> Vec<String> {
    match instant {
        InstantRecordingMeta::Complete { .. } => Vec::new(),
        InstantRecordingMeta::InProgress { recording } => {
            let state = if *recording {
                "still recording"
            } else {
                "incomplete"
            };
            vec![format!("instant recording is {state}")]
        }
        InstantRecordingMeta::Failed { error } => {
            vec![format!("instant recording failed: {error}")]
        }
    }
}

fn build_report(project_path: &Path, meta: &RecordingMeta) -> ValidationReport {
    let mut checks = vec![required_check(
        "recordingMeta",
        project_path.join("recording-meta.json"),
    )];
    checks.push(optional_check(
        "projectConfig",
        project_path.join("project-config.json"),
    ));

    let problems = match &meta.inner {
        RecordingMetaInner::Studio(studio) => {
            checks.extend(studio_checks(meta, studio));
            checks.push(optional_check("output", meta.output_path()));
            studio_problems(studio)
        }
        RecordingMetaInner::Instant(instant) => {
            checks.push(required_check("output", meta.output_path()));
            instant_problems(instant)
        }
    };

    let missing: Vec<PathBuf> = checks
        .iter()
        .filter(|c| c.required && !c.exists)
        .map(|c| c.path.clone())
        .collect();

    let valid = missing.is_empty() && problems.is_empty();
    let error = (!valid).then(|| {
        let mut reasons = Vec::new();
        if !missing.is_empty() {
            reasons.push(format!("missing {} required file(s)", missing.len()));
        }
        reasons.extend(problems.iter().cloned());
        format!("project validation failed: {}", reasons.join("; "))
    });

    ValidationReport {
        project_path: project_path.to_path_buf(),
        valid,
        duration_seconds: project_duration_seconds(project_path, meta),
        recording_type: Some(recording_type(meta)),
        error,
        checks,
        missing,
        problems,
    }
}

pub(crate) fn validate_project(project_path: &Path) -> Result<(), String> {
    let meta = RecordingMeta::load_for_project(project_path)
        .map_err(|e| format!("Failed to load recording meta: {e}"))?;
    let report = build_report(project_path, &meta);

    if report.valid {
        Ok(())
    } else {
        Err(report
            .error
            .unwrap_or_else(|| "project validation failed".to_string()))
    }
}

pub fn validate(project_path: PathBuf, format: OutputFormat) -> Result<(), String> {
    let report = match RecordingMeta::load_for_project(&project_path) {
        Ok(meta) => build_report(&project_path, &meta),
        Err(e) => ValidationReport {
            checks: vec![required_check(
                "recordingMeta",
                project_path.join("recording-meta.json"),
            )],
            missing: vec![project_path.join("recording-meta.json")],
            project_path: project_path.clone(),
            valid: false,
            duration_seconds: None,
            recording_type: None,
            error: Some(format!("Failed to load recording meta: {e}")),
            problems: Vec::new(),
        },
    };

    let valid = report.valid;

    match format {
        OutputFormat::Json => write_json(&report)?,
        OutputFormat::Text => {
            println!("project: {}", report.project_path.display());
            if let Some(duration) = report.duration_seconds {
                println!("duration: {duration:.2}s");
            }
            if let Some(error) = &report.error {
                println!("error: {error}");
            }
            for problem in &report.problems {
                println!("problem: {problem}");
            }
            for check in &report.checks {
                let status = if check.exists { "ok" } else { "missing" };
                let required = if check.required {
                    "required"
                } else {
                    "optional"
                };
                println!(
                    "  [{status}] {} ({required}): {}",
                    check.role,
                    check.path.display()
                );
            }
            println!("valid: {valid}");
        }
    }

    if valid {
        Ok(())
    } else {
        Err("project validation failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_schema_accepts_the_current_config_and_timeline() {
        let config = cap_project::ProjectConfiguration::default();
        parse_config(&serde_json::to_string(&config).unwrap()).unwrap();
        parse_config(r#"{"timeline":{"segments":[],"zoomSegments":[]}}"#).unwrap();
        let schema =
            serde_json::to_value(cap_project::ProjectConfiguration::json_schema()).unwrap();
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }

    #[test]
    fn generated_input_schema_preserves_legacy_aliases() {
        parse_config(r#"{"camera":{"zoom_size":1.5,"advanced_shadow":{"size":1,"opacity":1,"blur":1},"rounding_type":"squircle"}}"#).unwrap();
        for style in ["regular", "quick", "rapid"] {
            parse_config(&serde_json::json!({"cursor":{"animationStyle":style}}).to_string())
                .unwrap();
        }
        assert!(parse_config(r#"{"camera":{"zoomSize":1,"zoom_size":2}}"#).is_err());
    }

    #[test]
    fn schema_rejects_misspelled_fields_and_wrong_types() {
        for (document, field) in [
            (
                r#"{"timeline":{"segments":[],"zoomSegment":[]}}"#,
                "zoomSegment",
            ),
            (r#"{"backgound":{}}"#, "backgound"),
            (r#"{"screenMotionBlur":"heavy"}"#, "screenMotionBlur"),
        ] {
            assert!(parse_config(document).unwrap_err().contains(field));
        }
        assert!(
            parse_config(r#"{"background":{"paddng":1}}"#)
                .unwrap_err()
                .contains("/background/paddng")
        );
    }

    #[test]
    fn invalid_config_cannot_replace_an_existing_document() {
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join("project-config.json");
        std::fs::write(&path, b"original").unwrap();
        let error = config_set(
            project.path().to_path_buf(),
            r#"{"timeline":{"segments":[],"zoomSegment":[]}}"#,
            OutputFormat::Text,
        )
        .unwrap_err();
        assert!(error.contains("zoomSegment"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }
}
