use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

use super::models::{EnvironmentSet, VarSource};
use crate::infra::env_loader;

#[derive(Error, Debug)]
#[allow(dead_code)] // Used in Step 4+.
pub enum EnvironmentError {
    #[error("Named environment '{0}' not found in reqsmith_envs.yml")]
    NamedEnvNotFound(String),

    #[error("Failed to read reqsmith_envs.yml: {0}")]
    ReqsmithEnvsReadError(String),

    #[error("Failed to parse reqsmith_envs.yml: {0}")]
    ReqsmithEnvsParseError(String),
}

/// Schema for `reqsmith_envs.yml`.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ReqsmithEnvsFile {
    environments: BTreeMap<String, BTreeMap<String, String>>,
}

/// Resolve environment variables from layered sources.
///
/// Precedence (highest wins):
/// 1. CLI `--var` overrides
/// 2. Named environment from `reqsmith_envs.yml`
/// 3. `.env.<name>` file
/// 4. `.env` file
/// 5. OS environment (applied as fallback during interpolation, not here)
#[allow(dead_code)] // Used in Step 4+.
pub fn resolve_environment(
    cwd: &Path,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
) -> Result<EnvironmentSet, EnvironmentError> {
    let mut values = HashMap::new();
    let mut sources = HashMap::new();

    // Layer 4 (lowest loaded here): .env file
    let dotenv = env_loader::load_env(cwd);
    for (k, v) in dotenv {
        sources.insert(k.clone(), VarSource::DotEnv);
        values.insert(k, v);
    }

    // Layer 3: .env.<name> file (if env_name provided)
    if let Some(name) = env_name {
        let named_env = env_loader::load_named_env(cwd, name);
        for (k, v) in named_env {
            sources.insert(k.clone(), VarSource::DotEnvNamed(name.to_string()));
            values.insert(k, v);
        }
    }

    // Layer 2: reqsmith_envs.yml named environment
    if let Some(name) = env_name {
        let yml_vars = load_reqsmith_envs_yml(cwd, name)?;
        for (k, v) in yml_vars {
            sources.insert(k.clone(), VarSource::ReqsmithEnvsYml(name.to_string()));
            values.insert(k, v);
        }
    }

    // Layer 1 (highest): CLI --var overrides
    for (k, v) in cli_vars {
        sources.insert(k.clone(), VarSource::CliOverride);
        values.insert(k.clone(), v.clone());
    }

    Ok(EnvironmentSet { values, sources })
}

/// Augment an environment set with OS env fallbacks for any unresolved variables.
///
/// `required_vars` is the set of variable names referenced in the request template.
/// For each that is missing from `env.values`, we check `std::env::var()`.
#[allow(dead_code)] // Used in Step 4+.
pub fn apply_os_env_fallback(env: &mut EnvironmentSet, required_vars: &[String]) {
    for var_name in required_vars {
        if !env.values.contains_key(var_name)
            && let Ok(val) = std::env::var(var_name)
        {
            env.sources.insert(var_name.clone(), VarSource::OsEnv);
            env.values.insert(var_name.clone(), val);
        }
    }
}

/// Load a named environment from `reqsmith_envs.yml`.
///
/// Returns an empty map if `reqsmith_envs.yml` does not exist.
/// Returns an error if the file exists but the named environment is not found.
fn load_reqsmith_envs_yml(
    cwd: &Path,
    env_name: &str,
) -> Result<BTreeMap<String, String>, EnvironmentError> {
    let yml_path = cwd.join("reqsmith_envs.yml");
    if !yml_path.exists() {
        return Ok(BTreeMap::new());
    }

    let content = std::fs::read_to_string(&yml_path)
        .map_err(|e| EnvironmentError::ReqsmithEnvsReadError(e.to_string()))?;

    let file: ReqsmithEnvsFile = serde_yaml::from_str(&content)
        .map_err(|e| EnvironmentError::ReqsmithEnvsParseError(e.to_string()))?;

    file.environments
        .get(env_name)
        .cloned()
        .ok_or_else(|| EnvironmentError::NamedEnvNotFound(env_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_loads_dotenv() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "BASE_URL=http://localhost\n").unwrap();

        let env = resolve_environment(tmp.path(), None, &[]).unwrap();
        assert_eq!(env.values.get("BASE_URL").unwrap(), "http://localhost");
        assert_eq!(env.sources.get("BASE_URL").unwrap(), &VarSource::DotEnv);
    }

    #[test]
    fn resolve_named_env_overrides_dotenv() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "BASE_URL=http://localhost\n").unwrap();
        fs::write(
            tmp.path().join(".env.staging"),
            "BASE_URL=https://staging.example.com\n",
        )
        .unwrap();

        let env = resolve_environment(tmp.path(), Some("staging"), &[]).unwrap();
        assert_eq!(
            env.values.get("BASE_URL").unwrap(),
            "https://staging.example.com"
        );
        assert_eq!(
            env.sources.get("BASE_URL").unwrap(),
            &VarSource::DotEnvNamed("staging".into())
        );
    }

    #[test]
    fn resolve_reqsmith_envs_yml_overrides_dotenv_named() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env.staging"), "TOKEN=from-dotenv\n").unwrap();
        fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  staging:\n    TOKEN: from-yml\n",
        )
        .unwrap();

        let env = resolve_environment(tmp.path(), Some("staging"), &[]).unwrap();
        assert_eq!(env.values.get("TOKEN").unwrap(), "from-yml");
        assert_eq!(
            env.sources.get("TOKEN").unwrap(),
            &VarSource::ReqsmithEnvsYml("staging".into())
        );
    }

    #[test]
    fn resolve_cli_vars_override_everything() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "TOKEN=from-env\n").unwrap();
        fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  staging:\n    TOKEN: from-yml\n",
        )
        .unwrap();

        let cli_vars = vec![("TOKEN".into(), "from-cli".into())];
        let env = resolve_environment(tmp.path(), Some("staging"), &cli_vars).unwrap();
        assert_eq!(env.values.get("TOKEN").unwrap(), "from-cli");
        assert_eq!(env.sources.get("TOKEN").unwrap(), &VarSource::CliOverride);
    }

    #[test]
    fn resolve_missing_reqsmith_envs_yml_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        // No reqsmith_envs.yml, but requesting a named env — file doesn't exist, so no error
        let env = resolve_environment(tmp.path(), Some("staging"), &[]).unwrap();
        assert!(env.values.is_empty());
    }

    #[test]
    fn resolve_named_env_not_found_in_yml_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  production:\n    TOKEN: prod-token\n",
        )
        .unwrap();

        let result = resolve_environment(tmp.path(), Some("staging"), &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("staging"));
    }

    #[test]
    fn resolve_no_env_name_skips_named_layers() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "A=1\n").unwrap();
        fs::write(tmp.path().join(".env.staging"), "B=2\n").unwrap();
        fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  staging:\n    C: 3\n",
        )
        .unwrap();

        let env = resolve_environment(tmp.path(), None, &[]).unwrap();
        assert_eq!(env.values.get("A").unwrap(), "1");
        assert!(!env.values.contains_key("B"));
        assert!(!env.values.contains_key("C"));
    }

    #[test]
    fn resolve_merges_non_overlapping_vars() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "A=1\n").unwrap();
        fs::write(tmp.path().join(".env.staging"), "B=2\n").unwrap();
        fs::write(
            tmp.path().join("reqsmith_envs.yml"),
            "environments:\n  staging:\n    C: '3'\n",
        )
        .unwrap();
        let cli_vars = vec![("D".into(), "4".into())];

        let env = resolve_environment(tmp.path(), Some("staging"), &cli_vars).unwrap();
        assert_eq!(env.values.get("A").unwrap(), "1");
        assert_eq!(env.values.get("B").unwrap(), "2");
        assert_eq!(env.values.get("C").unwrap(), "3");
        assert_eq!(env.values.get("D").unwrap(), "4");
    }

    #[test]
    fn os_env_fallback_fills_missing_vars() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = resolve_environment(tmp.path(), None, &[]).unwrap();

        // SAFETY: Test-only, single-threaded test runner for this module.
        unsafe { std::env::set_var("REQSMITH_TEST_VAR_XYZ", "from-os") };
        apply_os_env_fallback(&mut env, &["REQSMITH_TEST_VAR_XYZ".into()]);
        assert_eq!(env.values.get("REQSMITH_TEST_VAR_XYZ").unwrap(), "from-os");
        assert_eq!(
            env.sources.get("REQSMITH_TEST_VAR_XYZ").unwrap(),
            &VarSource::OsEnv
        );
        unsafe { std::env::remove_var("REQSMITH_TEST_VAR_XYZ") };
    }

    #[test]
    fn os_env_fallback_does_not_override_existing() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".env"), "MY_VAR=from-dotenv\n").unwrap();
        let mut env = resolve_environment(tmp.path(), None, &[]).unwrap();

        // SAFETY: Test-only, single-threaded test runner for this module.
        unsafe { std::env::set_var("MY_VAR", "from-os") };
        apply_os_env_fallback(&mut env, &["MY_VAR".into()]);
        // Should keep the .env value, not the OS value
        assert_eq!(env.values.get("MY_VAR").unwrap(), "from-dotenv");
        unsafe { std::env::remove_var("MY_VAR") };
    }
}
