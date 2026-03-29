use std::collections::HashMap;
use std::path::Path;

#[allow(dead_code)] // Used in Step 5.
/// Load environment variables from a `.env` file in the given directory.
///
/// Returns an empty map if the file does not exist. Errors from parsing
/// are logged but do not prevent the app from starting.
pub fn load_env(cwd: &Path) -> HashMap<String, String> {
    let env_path = cwd.join(".env");
    if !env_path.exists() {
        return HashMap::new();
    }

    match dotenvy::from_path_iter(&env_path) {
        Ok(iter) => iter
            .filter_map(|result| match result {
                Ok((key, value)) => Some((key, value)),
                Err(e) => {
                    tracing::warn!("Skipping malformed .env entry: {e}");
                    None
                }
            })
            .collect(),
        Err(e) => {
            tracing::warn!("Failed to read .env file: {e}");
            HashMap::new()
        }
    }
}

/// Load environment variables from a `.env.<name>` file in the given directory.
///
/// Returns an empty map if the file does not exist.
#[allow(dead_code)] // Used by environment module in Step 3+.
pub fn load_named_env(cwd: &Path, name: &str) -> HashMap<String, String> {
    let env_path = cwd.join(format!(".env.{name}"));
    if !env_path.exists() {
        return HashMap::new();
    }

    match dotenvy::from_path_iter(&env_path) {
        Ok(iter) => iter
            .filter_map(|result| match result {
                Ok((key, value)) => Some((key, value)),
                Err(e) => {
                    tracing::warn!("Skipping malformed .env.{name} entry: {e}");
                    None
                }
            })
            .collect(),
        Err(e) => {
            tracing::warn!("Failed to read .env.{name} file: {e}");
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_env_returns_empty_when_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vars = load_env(tmp.path());
        assert!(vars.is_empty());
    }

    #[test]
    fn load_env_parses_simple_env_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(".env"),
            "BASE_URL=https://api.example.com\nTOKEN=secret123\n",
        )
        .unwrap();

        let vars = load_env(tmp.path());
        assert_eq!(vars.get("BASE_URL").unwrap(), "https://api.example.com");
        assert_eq!(vars.get("TOKEN").unwrap(), "secret123");
    }

    #[test]
    fn load_env_handles_quoted_values() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "MSG=\"hello world\"\n").unwrap();

        let vars = load_env(tmp.path());
        assert_eq!(vars.get("MSG").unwrap(), "hello world");
    }

    #[test]
    fn load_named_env_returns_empty_when_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vars = load_named_env(tmp.path(), "staging");
        assert!(vars.is_empty());
    }

    #[test]
    fn load_named_env_reads_named_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(".env.staging"),
            "BASE_URL=https://staging.example.com\n",
        )
        .unwrap();

        let vars = load_named_env(tmp.path(), "staging");
        assert_eq!(vars.get("BASE_URL").unwrap(), "https://staging.example.com");
    }

    #[test]
    fn load_env_skips_comments() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(".env"),
            "# This is a comment\nKEY=value\n# Another comment\n",
        )
        .unwrap();

        let vars = load_env(tmp.path());
        assert_eq!(vars.len(), 1);
        assert_eq!(vars.get("KEY").unwrap(), "value");
    }
}
