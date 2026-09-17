use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};

use super::atomic_write;
use super::models::StoredRun;
use super::redaction;

/// Save a run snapshot to the `.reqsmith/runs/` directory.
///
/// Headers are redacted here, at the write boundary, so a secret returned by a
/// server (e.g. `Set-Cookie`) cannot reach disk no matter who built the
/// snapshot. The run is taken by value so this costs no copy of the body. The
/// write itself is atomic, symlink-refusing, and race-free against a concurrent
/// save picking the same name — see [`atomic_write`] and
/// [`write_unique_snapshot`].
pub fn save_run(dir: &Path, mut run: StoredRun) -> Result<PathBuf> {
    let runs_dir = ensure_reqsmith_subdir(dir, "runs")?;
    run.headers = redaction::redact_headers(&run.headers);
    let json = serde_json::to_string_pretty(&run).wrap_err("Failed to serialize run")?;

    write_unique_snapshot(
        &runs_dir,
        &sanitize_stem(&format!("{}_{}", run.request_name, run.timestamp)),
        "json",
        json.as_bytes(),
    )
}

/// Reduce a filename stem to `[A-Za-z0-9_-]`, so a request name or timestamp
/// carrying `/` or `..` can never steer the write outside `.reqsmith/`.
fn sanitize_stem(raw: &str) -> String {
    raw.replace(
        |c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_',
        "_",
    )
}

/// Save a binary response body to `.reqsmith/downloads/` with a collision-safe filename.
pub fn save_response_body(bytes: &[u8], dir: &Path) -> Result<PathBuf> {
    let downloads_dir = ensure_reqsmith_subdir(dir, "downloads")?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    write_unique_snapshot(
        &downloads_dir,
        &format!("response_{timestamp}"),
        "bin",
        bytes,
    )
}

/// Create (if needed) `<base>/.reqsmith/<sub>` and refuse if either that directory
/// or the intermediate `.reqsmith` directory is a symlink.
///
/// This is a cheap backstop against the obvious `.reqsmith -> /somewhere/hostile`
/// (or `runs -> …`) swap that would otherwise let a write land outside the
/// project tree. It is checked before creating anything, so we never
/// `create_dir_all` *through* a symlinked `.reqsmith`. A fully attacker-controlled
/// parent tree is out of scope (see the note in [`atomic_write`]).
fn ensure_reqsmith_subdir(base: &Path, sub: &str) -> Result<PathBuf> {
    let reqsmith_dir = base.join(".reqsmith");
    if is_symlink(&reqsmith_dir) {
        return Err(eyre!(
            "Refusing to use symlinked .reqsmith directory at {}",
            reqsmith_dir.display()
        ));
    }

    let dir = reqsmith_dir.join(sub);
    std::fs::create_dir_all(&dir)
        .wrap_err_with(|| format!("Failed to create {} directory", dir.display()))?;
    if is_symlink(&dir) {
        return Err(eyre!(
            "Refusing to use symlinked directory at {}",
            dir.display()
        ));
    }
    Ok(dir)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Write `data` to a fresh `{stem}.{ext}` file (or `{stem}_{n}.{ext}` on
/// collision) in `dir`, reserving each candidate atomically so two concurrent
/// saves never clobber each other. A symlink at a candidate path is fatal
/// (never followed), unlike a plain collision which advances to the next name.
fn write_unique_snapshot(dir: &Path, stem: &str, extension: &str, data: &[u8]) -> Result<PathBuf> {
    const MAX_CANDIDATES: usize = 10_000;
    for suffix in 0..=MAX_CANDIDATES {
        let candidate = if suffix == 0 {
            dir.join(format!("{stem}.{extension}"))
        } else {
            dir.join(format!("{stem}_{suffix}.{extension}"))
        };
        match atomic_write::write_new(&candidate, data) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(eyre!("Failed to write {}: {e}", candidate.display()));
            }
        }
    }
    Err(eyre!(
        "Exhausted {} filename candidates for '{stem}.{extension}' under {}",
        MAX_CANDIDATES,
        dir.display()
    ))
}

/// Load a stored run from a JSON file.
pub fn load_run(path: &Path) -> Result<StoredRun> {
    let content = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("Failed to read run file: {}", path.display()))?;
    // A crash between reserving a snapshot name and the rename leaves a 0-byte
    // file behind; name that plainly instead of a confusing JSON parse error.
    if content.trim().is_empty() {
        return Err(eyre!(
            "Run file is empty (an interrupted write can leave a 0-byte snapshot; delete it): {}",
            path.display()
        ));
    }
    let run: StoredRun = serde_json::from_str(&content)
        .wrap_err_with(|| format!("Failed to parse run file: {}", path.display()))?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{
        AssertionReport, ExitCode, HeaderDiff, ResponseArtifact, RunResult, StoredRun,
    };
    use crate::core::redaction;

    /// A run result carrying `headers`, so tests exercise the real
    /// `RunResult -> StoredRun` path where redaction happens.
    fn run_result(headers: &[(&str, &str)]) -> RunResult {
        RunResult {
            request_name: "Get Users".into(),
            request_file: PathBuf::from("requests/get_users.req.yml"),
            response: Some(ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                content_type: Some("application/json".into()),
                content_length: None,
                duration_ms: 150,
                body_text: Some(r#"{"users":[]}"#.into()),
                body_bytes: None,
                is_binary: false,
                truncated: false,
            }),
            error: None,
            exit_code: ExitCode::Success,
            cancelled: false,
            assertions: AssertionReport::default(),
        }
    }

    fn sample_run() -> StoredRun {
        StoredRun {
            request_name: "Get Users".into(),
            request_file: PathBuf::from("requests/get_users.req.yml"),
            timestamp: "1711700000".into(),
            status_code: 200,
            duration_ms: 150,
            headers: vec![("content-type".into(), "application/json".into())],
            content_type: Some("application/json".into()),
            body_text: Some(r#"{"users":[]}"#.into()),
            truncated: false,
            assertions: vec![],
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();

        let path = save_run(tmp.path(), run.clone()).unwrap();
        assert!(path.exists());

        let loaded = load_run(&path).unwrap();
        assert_eq!(loaded.request_name, run.request_name);
        assert_eq!(loaded.status_code, run.status_code);
        assert_eq!(loaded.body_text, run.body_text);
    }

    #[test]
    fn load_run_reports_an_empty_snapshot_clearly() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("interrupted.json");
        std::fs::write(&path, b"").unwrap();

        let err = load_run(&path).unwrap_err().to_string();
        assert!(err.contains("empty"), "unhelpful error: {err}");
        assert!(!err.contains("expected value"), "raw parse error: {err}");
    }

    #[test]
    fn sanitize_stem_reduces_the_whole_stem() {
        assert_eq!(sanitize_stem("Get Users_2024/01"), "Get_Users_2024_01");
        assert_eq!(sanitize_stem("../etc/passwd_..%2f"), "___etc_passwd____2f");
        assert_eq!(sanitize_stem("café_東京"), "caf____");
    }

    #[test]
    fn save_run_sanitizes_the_timestamp_too() {
        let tmp = tempfile::tempdir().unwrap();
        let mut run = sample_run();
        run.timestamp = "../../evil".into();

        let path = save_run(tmp.path(), run).unwrap();
        assert_eq!(
            path.parent().unwrap(),
            tmp.path().join(".reqsmith").join("runs").as_path()
        );
        assert!(!path.to_string_lossy().contains(".."));
    }

    #[test]
    fn directory_created_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();
        let path = save_run(tmp.path(), run).unwrap();
        assert!(path.parent().unwrap().exists());
    }

    #[test]
    fn save_run_avoids_overwriting_existing_file_on_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();

        let first = save_run(tmp.path(), run.clone()).unwrap();
        let second = save_run(tmp.path(), run).unwrap();

        assert_ne!(first, second);
        assert!(first.exists());
        assert!(second.exists());
    }

    #[test]
    fn stored_run_mirrors_the_truncation_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let mut result = run_result(&[("content-type", "application/json")]);
        if let Some(response) = result.response.as_mut() {
            response.truncated = true;
        }

        let run = StoredRun::from_run_result(&result).unwrap();
        assert!(run.truncated);

        let path = save_run(tmp.path(), run).unwrap();
        assert!(load_run(&path).unwrap().to_response_artifact().truncated);
    }

    #[test]
    fn stored_run_json_round_trip() {
        let run = sample_run();
        let json = serde_json::to_string(&run).unwrap();
        let parsed: StoredRun = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.request_name, run.request_name);
        assert_eq!(parsed.status_code, run.status_code);
    }

    #[test]
    fn save_response_body_writes_binary_download() {
        let tmp = tempfile::tempdir().unwrap();
        let path = save_response_body(&[0_u8, 1, 2, 3], tmp.path()).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), vec![0_u8, 1, 2, 3]);
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("bin"));
    }

    // --- stored-run header redaction ---

    #[test]
    fn save_run_redacts_sensitive_headers_before_persisting() {
        let tmp = tempfile::tempdir().unwrap();
        let run = StoredRun::from_run_result(&run_result(&[
            ("authorization", "Bearer super-secret"),
            ("content-type", "application/json"),
            ("set-cookie", "session=abc123"),
        ]))
        .unwrap();

        let path = save_run(tmp.path(), run).unwrap();

        // The raw bytes on disk must never contain the secret values.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("super-secret"));
        assert!(!raw.contains("abc123"));
        assert!(raw.contains(redaction::REDACTED_PREFIX));

        let loaded = load_run(&path).unwrap();
        let value_of = |name: &str| {
            loaded
                .headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert!(value_of("authorization").starts_with(redaction::REDACTED_PREFIX));
        assert!(value_of("set-cookie").starts_with(redaction::REDACTED_PREFIX));
        assert_eq!(value_of("content-type"), "application/json");
    }

    #[test]
    fn diffing_two_stored_runs_still_reports_a_rotated_cookie() {
        let tmp = tempfile::tempdir().unwrap();

        let first =
            StoredRun::from_run_result(&run_result(&[("set-cookie", "session=old-secret")]))
                .unwrap();
        let mut second =
            StoredRun::from_run_result(&run_result(&[("set-cookie", "session=new-secret")]))
                .unwrap();
        second.timestamp = "1711700001".into();

        let first_path = save_run(tmp.path(), first).unwrap();
        let second_path = save_run(tmp.path(), second).unwrap();

        let baseline = load_run(&first_path).unwrap().to_response_artifact();
        let candidate = load_run(&second_path).unwrap().to_response_artifact();
        let diff = crate::core::diffing::diff_responses(&baseline, &candidate, "old", "new");

        assert_eq!(diff.header_diffs.len(), 1);
        let HeaderDiff::Changed { key, old, new } = &diff.header_diffs[0] else {
            panic!("expected a changed header, got {:?}", diff.header_diffs[0]);
        };
        assert_eq!(key, "set-cookie");
        assert_ne!(old, new);
        for value in [old, new] {
            assert!(value.starts_with(redaction::REDACTED_PREFIX));
            assert!(!value.contains("old-secret"));
            assert!(!value.contains("new-secret"));
        }

        // Redaction must be idempotent: the diff's old/new must be exactly
        // the placeholders already persisted on disk, not a re-hash of them
        // (re-hashing a placeholder would make `reqsmith diff` show a
        // fingerprint that doesn't match what's in the files).
        let persisted_value = |path: &Path| {
            load_run(path)
                .unwrap()
                .headers
                .into_iter()
                .find(|(k, _)| k == "set-cookie")
                .map(|(_, v)| v)
                .unwrap()
        };
        assert_eq!(old, &persisted_value(&first_path));
        assert_eq!(new, &persisted_value(&second_path));
    }

    // --- path traversal and symlink safety ---

    #[test]
    fn save_run_sanitizes_path_traversal_in_request_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mut run = sample_run();
        run.request_name = "../../etc/passwd".into();

        let path = save_run(tmp.path(), run).unwrap();
        let runs_dir = tmp.path().join(".reqsmith").join("runs");

        assert_eq!(path.parent().unwrap(), runs_dir.as_path());
        assert!(path.exists());
        assert!(!path.to_string_lossy().contains(".."));
    }

    #[test]
    fn save_run_sanitizes_slashes_in_request_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mut run = sample_run();
        run.request_name = "nested/name/with/slashes".into();

        let path = save_run(tmp.path(), run).unwrap();
        let runs_dir = tmp.path().join(".reqsmith").join("runs");

        assert_eq!(path.parent().unwrap(), runs_dir.as_path());
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_run_refuses_to_follow_a_preexisting_symlink_at_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();
        let runs_dir = tmp.path().join(".reqsmith").join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();

        let stem = sanitize_stem(&format!("{}_{}", run.request_name, run.timestamp));
        let target = runs_dir.join(format!("{stem}.json"));

        // A dangling symlink: its target does not exist, so `Path::exists()`
        // (used by `write_unique_snapshot` for collision detection) reports
        // this path as "free" — exactly the gap `atomic_write::write_new` must close.
        std::os::unix::fs::symlink(runs_dir.join("does-not-exist"), &target).unwrap();

        let result = save_run(tmp.path(), run);
        assert!(
            result.is_err(),
            "expected save_run to refuse a pre-existing symlink destination"
        );

        // The symlink itself must be left untouched, not followed/replaced.
        let meta = std::fs::symlink_metadata(&target).unwrap();
        assert!(meta.file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn save_run_refuses_symlinked_reqsmith_parent_directory() {
        // A hostile `.reqsmith` pointing elsewhere must not let a snapshot write
        // escape the project tree.
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), tmp.path().join(".reqsmith")).unwrap();

        let result = save_run(tmp.path(), sample_run());
        assert!(
            result.is_err(),
            "save_run must refuse a symlinked .reqsmith directory"
        );
        // Nothing was written into the symlink target.
        assert!(!elsewhere.path().join("runs").exists());
    }

    #[test]
    fn concurrent_saves_never_clobber_each_other() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let base = Arc::new(tmp.path().to_path_buf());

        // Every thread saves a run with the SAME request_name+timestamp, so
        // they all contend for the identical primary filename. Race-free
        // reservation must hand each a distinct path.
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let base = Arc::clone(&base);
                thread::spawn(move || {
                    let mut run = sample_run();
                    run.body_text = Some(format!("{{\"n\":{i}}}"));
                    save_run(&base, run).unwrap()
                })
            })
            .collect();

        let paths: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            paths.len(),
            "two saves collided onto one path"
        );
        // Every saved file still exists (none was overwritten out of existence).
        assert!(paths.iter().all(|p| p.exists()));
    }
}
