use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};

use super::atomic_write;
use super::models::StoredRun;
use super::redaction;

/// Save a run snapshot to the `.reqsmith/runs/` directory.
///
/// Response headers are redacted (see [`redaction::redact_headers`]) before
/// the snapshot is serialized, so secrets returned by a server (e.g.
/// `Set-Cookie`, a bearer token echoed back in a header) never land on disk.
/// The write itself is atomic, symlink-refusing, and race-free against a
/// concurrent save picking the same name — see [`atomic_write`] and
/// [`write_unique_snapshot`].
pub fn save_run(run: &StoredRun, dir: &Path) -> Result<PathBuf> {
    let runs_dir = ensure_reqsmith_subdir(dir, "runs")?;

    let sanitized_name = run
        .request_name
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");

    let mut redacted = run.clone();
    redacted.headers = redaction::redact_headers(&redacted.headers);
    let json = serde_json::to_string_pretty(&redacted).wrap_err("Failed to serialize run")?;

    write_unique_snapshot(
        &runs_dir,
        &format!("{}_{}", sanitized_name, run.timestamp),
        "json",
        json.as_bytes(),
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
    let run: StoredRun = serde_json::from_str(&content)
        .wrap_err_with(|| format!("Failed to parse run file: {}", path.display()))?;
    Ok(run)
}

/// List stored run files for a given request name.
pub fn list_runs(dir: &Path, request_name: &str) -> Result<Vec<PathBuf>> {
    let runs_dir = dir.join(".reqsmith").join("runs");
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }

    let sanitized_prefix =
        request_name.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&runs_dir)
        .wrap_err("Failed to read .reqsmith/runs directory")?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "json")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(&sanitized_prefix))
        })
        .collect();

    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::StoredRun;

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
            assertions: vec![],
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();

        let path = save_run(&run, tmp.path()).unwrap();
        assert!(path.exists());

        let loaded = load_run(&path).unwrap();
        assert_eq!(loaded.request_name, run.request_name);
        assert_eq!(loaded.status_code, run.status_code);
        assert_eq!(loaded.body_text, run.body_text);
    }

    #[test]
    fn list_runs_finds_matching_files() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();

        save_run(&run, tmp.path()).unwrap();
        let mut run2 = sample_run();
        run2.timestamp = "1711700001".into();
        save_run(&run2, tmp.path()).unwrap();

        let paths = list_runs(tmp.path(), "Get Users").unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn list_runs_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = list_runs(tmp.path(), "whatever").unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn directory_created_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();
        let path = save_run(&run, tmp.path()).unwrap();
        assert!(path.parent().unwrap().exists());
    }

    #[test]
    fn save_run_avoids_overwriting_existing_file_on_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let run = sample_run();

        let first = save_run(&run, tmp.path()).unwrap();
        let second = save_run(&run, tmp.path()).unwrap();

        assert_ne!(first, second);
        assert!(first.exists());
        assert!(second.exists());
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

    // --- B1: stored-run header redaction ---

    #[test]
    fn save_run_redacts_sensitive_headers_before_persisting() {
        let tmp = tempfile::tempdir().unwrap();
        let mut run = sample_run();
        run.headers = vec![
            ("authorization".into(), "Bearer super-secret".into()),
            ("content-type".into(), "application/json".into()),
            ("set-cookie".into(), "session=abc123".into()),
        ];

        let path = save_run(&run, tmp.path()).unwrap();

        // The raw bytes on disk must never contain the secret values.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("super-secret"));
        assert!(!raw.contains("abc123"));
        assert!(raw.contains("<redacted>"));

        let loaded = load_run(&path).unwrap();
        let auth = loaded
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .unwrap();
        assert_eq!(auth.1, "<redacted>");
        let cookie = loaded
            .headers
            .iter()
            .find(|(k, _)| k == "set-cookie")
            .unwrap();
        assert_eq!(cookie.1, "<redacted>");
        let ct = loaded
            .headers
            .iter()
            .find(|(k, _)| k == "content-type")
            .unwrap();
        assert_eq!(ct.1, "application/json");
    }

    // --- B3: path traversal and symlink safety ---

    #[test]
    fn save_run_sanitizes_path_traversal_in_request_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mut run = sample_run();
        run.request_name = "../../etc/passwd".into();

        let path = save_run(&run, tmp.path()).unwrap();
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

        let path = save_run(&run, tmp.path()).unwrap();
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

        let sanitized_name = run
            .request_name
            .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
        let target = runs_dir.join(format!("{}_{}.json", sanitized_name, run.timestamp));

        // A dangling symlink: its target does not exist, so `Path::exists()`
        // (used by `next_available_path` for collision detection) reports
        // this path as "free" — exactly the gap `write_atomically` must close.
        std::os::unix::fs::symlink(runs_dir.join("does-not-exist"), &target).unwrap();

        let result = save_run(&run, tmp.path());
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

        let result = save_run(&sample_run(), tmp.path());
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
                    save_run(&run, &base).unwrap()
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
