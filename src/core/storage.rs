use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result};

use super::models::StoredRun;

/// Save a run snapshot to the `.hurl/runs/` directory.
pub fn save_run(run: &StoredRun, dir: &Path) -> Result<PathBuf> {
    let runs_dir = dir.join(".hurl").join("runs");
    std::fs::create_dir_all(&runs_dir).wrap_err("Failed to create .hurl/runs directory")?;

    let sanitized_name = run
        .request_name
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    let path = next_available_path(
        &runs_dir,
        &format!("{}_{}", sanitized_name, run.timestamp),
        "json",
    );

    let json = serde_json::to_string_pretty(run).wrap_err("Failed to serialize run")?;
    std::fs::write(&path, json).wrap_err("Failed to write run file")?;

    Ok(path)
}

/// Save a binary response body to `.hurl/downloads/` with a collision-safe filename.
pub fn save_response_body(bytes: &[u8], dir: &Path) -> Result<PathBuf> {
    let downloads_dir = dir.join(".hurl").join("downloads");
    std::fs::create_dir_all(&downloads_dir)
        .wrap_err("Failed to create .hurl/downloads directory")?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = next_available_path(&downloads_dir, &format!("response_{timestamp}"), "bin");
    std::fs::write(&path, bytes).wrap_err("Failed to write response body")?;
    Ok(path)
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
    let runs_dir = dir.join(".hurl").join("runs");
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }

    let sanitized_prefix =
        request_name.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&runs_dir)
        .wrap_err("Failed to read .hurl/runs directory")?
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

fn next_available_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{extension}"));
    if !path.exists() {
        return path;
    }

    let mut suffix = 1usize;
    loop {
        path = dir.join(format!("{stem}_{suffix}.{extension}"));
        if !path.exists() {
            return path;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::StoredRun;

    fn sample_run() -> StoredRun {
        StoredRun {
            request_name: "Get Users".into(),
            request_file: PathBuf::from("requests/get_users.hurl.yml"),
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
}
