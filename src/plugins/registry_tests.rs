use super::*;
use std::path::PathBuf;

/// Spins forever, so the wall-clock timeout is the only way out.
const SPIN_WAT: &str = r#"
(module
  (func (export "run") (result i32)
    (loop $spin (br $spin))
    (i32.const 0)))
"#;

/// Traps immediately.
const TRAP_WAT: &str = r#"
(module
  (func (export "run") (result i32)
    unreachable))
"#;

/// Echoes its input back as output.
const ECHO_WAT: &str = r#"
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (func (export "run") (result i32)
    (call $output_set (call $input_offset) (call $length (call $input_offset)))
    (i32.const 0)))
"#;

/// Treats its input as an env var name, asks the host for it, and writes
/// whatever comes back as output — exercising the `filter_env` wiring.
const READ_ENV_WAT: &str = r#"
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "provide_env_var" (func $provide_env_var (param i64) (result i64)))
  (func (export "run") (result i32)
    (local $value i64)
    (local.set $value (call $provide_env_var (call $input_offset)))
    (call $output_set (local.get $value) (call $length (local.get $value)))
    (i32.const 0)))
"#;

fn write_project_config(root: &Path, toml: &str) -> PathBuf {
    let dir = root.join(".reqsmith");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugins.toml"), toml).unwrap();
    dir
}

fn write_wasm(dir: &Path, name: &str, wat: &str) {
    std::fs::write(dir.join(name), wat::parse_str(wat).unwrap()).unwrap();
}

fn discovered_stub(path: PathBuf, source: ConfigSource) -> DiscoveredConfig {
    DiscoveredConfig {
        path,
        config: PluginsConfig {
            api_version: 1,
            timeout_ms: 5000,
            memory_limit_pages: 256,
            plugin: vec![],
        },
        source,
    }
}

#[test]
fn discover_returns_empty_when_no_config() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, false);
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.plugin_names().is_empty());
    assert!(registry.blocked_project_config().is_none());
}

#[test]
fn discover_skips_missing_wasm_files() {
    let tmp = tempfile::tempdir().unwrap();
    write_project_config(
        tmp.path(),
        r#"
api_version = 1
[[plugin]]
name = "ghost"
path = "nonexistent.wasm"
capabilities = ["pre_request"]
"#,
    );

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    assert!(registry.is_empty());
    assert!(registry.load_errors().is_empty());
}

#[test]
fn discover_records_invalid_api_version_as_a_config_error() {
    // This used to hard-abort discovery via `?`; it is now a
    // per-config error that leaves the rest of the registry buildable.
    let tmp = tempfile::tempdir().unwrap();
    write_project_config(
        tmp.path(),
        r#"
api_version = 99
[[plugin]]
name = "test"
path = "test.wasm"
capabilities = ["pre_request"]
"#,
    );

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    assert!(registry.is_empty());
    assert_eq!(registry.config_errors().len(), 1);
    assert!(registry.config_errors()[0].1.to_string().contains("99"));
}

#[test]
fn resolve_wasm_path_uses_config_directory_for_relative_paths() {
    let discovered = discovered_stub(
        PathBuf::from("/tmp/project/.reqsmith/plugins.toml"),
        ConfigSource::ProjectLocal,
    );

    let resolved = resolve_wasm_path(
        &discovered,
        Path::new("/tmp/project"),
        Path::new("plugins/a.wasm"),
    )
    .unwrap();

    assert_eq!(
        resolved,
        PathBuf::from("/tmp/project/.reqsmith/plugins/a.wasm")
    );
}

#[test]
fn resolve_wasm_path_rejects_paths_escaping_the_config_directory() {
    let discovered = discovered_stub(
        PathBuf::from("/tmp/project/.reqsmith/plugins.toml"),
        ConfigSource::ProjectLocal,
    );

    let bad_paths = vec![
        std::env::temp_dir().join("evil.wasm"),
        PathBuf::from("../../evil.wasm"),
        PathBuf::from("~/evil.wasm"),
    ];
    #[cfg(windows)]
    let bad_paths = {
        let mut bad_paths = bad_paths;
        bad_paths.extend([
            PathBuf::from(r"\etc\evil.wasm"),
            PathBuf::from(r"C:\etc\evil.wasm"),
            PathBuf::from(r"C:evil.wasm"),
        ]);
        bad_paths
    };

    for bad in bad_paths {
        let err = resolve_wasm_path(&discovered, Path::new("/tmp/project"), &bad).unwrap_err();
        assert!(
            err.to_string().contains("must live under"),
            "unexpected message for {bad:?}: {err}"
        );
    }
}

// A textually-relative path can still escape via a symlink, since
// `is_file()` follows symlinks and the old check never canonicalized.
#[cfg(unix)]
#[test]
fn resolve_wasm_path_rejects_a_symlink_escaping_the_config_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join(".reqsmith");
    std::fs::create_dir_all(&config_dir).unwrap();

    let outside_dir = tmp.path().join("outside");
    std::fs::create_dir_all(&outside_dir).unwrap();
    let outside_file = outside_dir.join("evil.wasm");
    std::fs::write(&outside_file, b"evil bytes").unwrap();

    std::os::unix::fs::symlink(&outside_file, config_dir.join("linked.wasm")).unwrap();

    let discovered = discovered_stub(config_dir.join("plugins.toml"), ConfigSource::ProjectLocal);
    let err = resolve_wasm_path(&discovered, tmp.path(), Path::new("linked.wasm")).unwrap_err();
    assert!(err.to_string().contains("outside"), "{err}");
}

// ------------------------------------------------------------------
// C1: project-local consent gate
// ------------------------------------------------------------------

#[test]
fn c1_gate_project_local_decision_table() {
    assert!(gate_project_local(ConfigSource::ProjectLocal, false));
    assert!(!gate_project_local(ConfigSource::ProjectLocal, true));
    // User-global is never gated, regardless of the opt-in flag.
    assert!(!gate_project_local(ConfigSource::UserGlobal, false));
    assert!(!gate_project_local(ConfigSource::UserGlobal, true));
}

/// A project + user-global pair whose entries are distinguishable by the
/// per-entry load error each one produces (both point at non-WASM bytes).
fn both_configs() -> (tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    let config_home = tempfile::tempdir().unwrap();

    let project_dir = write_project_config(
        project.path(),
        r#"
api_version = 1
[[plugin]]
name = "from-project"
path = "p.wasm"
capabilities = ["pre_request"]
"#,
    );
    std::fs::write(project_dir.join("p.wasm"), b"not wasm").unwrap();

    let global_dir = config_home.path().join("reqsmith");
    std::fs::create_dir_all(&global_dir).unwrap();
    std::fs::write(
        global_dir.join("plugins.toml"),
        r#"
api_version = 1
[[plugin]]
name = "from-global"
path = "g.wasm"
capabilities = ["pre_request"]
"#,
    )
    .unwrap();
    std::fs::write(global_dir.join("g.wasm"), b"not wasm").unwrap();

    (project, config_home)
}

#[test]
fn c1_blocked_project_local_still_loads_user_global() {
    let (project, config_home) = both_configs();

    let registry =
        PluginRegistry::discover_and_load_with(project.path(), Some(config_home.path()), false);

    let attempted: Vec<&str> = registry
        .load_errors()
        .iter()
        .map(|failure| failure.name.as_str())
        .collect();
    assert_eq!(attempted, vec!["from-global"]);
    assert_eq!(
        registry.blocked_project_config(),
        Some(project.path().join(".reqsmith/plugins.toml").as_path())
    );
}

#[test]
fn c1_opt_in_attempts_both_configs() {
    let (project, config_home) = both_configs();

    let registry =
        PluginRegistry::discover_and_load_with(project.path(), Some(config_home.path()), true);

    let mut attempted: Vec<&str> = registry
        .load_errors()
        .iter()
        .map(|failure| failure.name.as_str())
        .collect();
    attempted.sort_unstable();
    assert_eq!(attempted, vec!["from-global", "from-project"]);
    assert!(registry.blocked_project_config().is_none());
}

// ------------------------------------------------------------------
// per-config discovery fault tolerance
// ------------------------------------------------------------------

#[test]
fn invalid_project_local_config_still_loads_user_global_plugins() {
    let project = tempfile::tempdir().unwrap();
    let config_home = tempfile::tempdir().unwrap();

    write_project_config(project.path(), "not valid toml {{{{");

    let global_dir = config_home.path().join("reqsmith");
    std::fs::create_dir_all(&global_dir).unwrap();
    std::fs::write(
        global_dir.join("plugins.toml"),
        r#"
api_version = 1
[[plugin]]
name = "from-global"
path = "g.wasm"
capabilities = ["pre_request"]
"#,
    )
    .unwrap();
    std::fs::write(global_dir.join("g.wasm"), b"not wasm").unwrap();

    let registry =
        PluginRegistry::discover_and_load_with(project.path(), Some(config_home.path()), true);

    // The global entry was still attempted (and failed only because
    // "not wasm" isn't valid WASM, not because discovery aborted).
    let attempted: Vec<&str> = registry
        .load_errors()
        .iter()
        .map(|failure| failure.name.as_str())
        .collect();
    assert_eq!(attempted, vec!["from-global"]);

    assert_eq!(registry.config_errors().len(), 1);
    assert_eq!(
        registry.config_errors()[0].0,
        project.path().join(".reqsmith/plugins.toml")
    );
}

// ------------------------------------------------------------------
// duplicate plugin names across configs
// ------------------------------------------------------------------

#[test]
fn duplicate_name_across_configs_loads_once_and_records_one_error() {
    let project = tempfile::tempdir().unwrap();
    let config_home = tempfile::tempdir().unwrap();

    let project_dir = write_project_config(
        project.path(),
        r#"
api_version = 1
[[plugin]]
name = "dup"
path = "p.wasm"
capabilities = ["pre_request"]
"#,
    );
    write_wasm(&project_dir, "p.wasm", ECHO_WAT);

    let global_dir = config_home.path().join("reqsmith");
    std::fs::create_dir_all(&global_dir).unwrap();
    std::fs::write(
        global_dir.join("plugins.toml"),
        r#"
api_version = 1
[[plugin]]
name = "dup"
path = "g.wasm"
capabilities = ["pre_request"]
"#,
    )
    .unwrap();
    write_wasm(&global_dir, "g.wasm", ECHO_WAT);

    let registry =
        PluginRegistry::discover_and_load_with(project.path(), Some(config_home.path()), true);

    assert_eq!(registry.plugin_names(), vec!["dup"]);
    assert_eq!(registry.load_errors().len(), 1);
    let failure = &registry.load_errors()[0];
    assert_eq!(failure.name, "dup");
    assert_eq!(
        failure.config_path,
        project.path().join(".reqsmith/plugins.toml")
    );
    assert!(
        failure.error.to_string().contains("already defined"),
        "{}",
        failure.error
    );
    assert!(
        failure.error.to_string().contains("plugins.toml"),
        "should name the config that first claimed it: {}",
        failure.error
    );
}

// ------------------------------------------------------------------
// Per-entry failure isolation
// ------------------------------------------------------------------

#[test]
fn a_failing_entry_does_not_abort_the_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_project_config(
        tmp.path(),
        &format!(
            r#"
api_version = 1
[[plugin]]
name = "tampered"
path = "tampered.wasm"
capabilities = ["pre_request"]
sha256 = "{}"

[[plugin]]
name = "absent"
path = "absent.wasm"
capabilities = ["pre_request"]
"#,
            "a".repeat(64)
        ),
    );
    std::fs::write(dir.join("tampered.wasm"), b"different bytes").unwrap();

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);

    assert_eq!(registry.load_errors().len(), 1);
    let failure = &registry.load_errors()[0];
    assert_eq!(failure.name, "tampered");
    assert!(matches!(
        failure.error,
        PluginError::IntegrityMismatch { .. }
    ));
    // The second (merely missing) entry is a skip, not an error, and the
    // first entry's failure did not prevent us reaching it.
    assert!(registry.is_empty());
}

#[test]
fn an_unreadable_module_is_a_per_entry_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_project_config(
        tmp.path(),
        r#"
api_version = 1
[[plugin]]
name = "escaping"
path = "../outside.wasm"
capabilities = ["pre_request"]
"#,
    );

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    assert_eq!(registry.load_errors().len(), 1);
    assert_eq!(registry.load_errors()[0].name, "escaping");
}

// ------------------------------------------------------------------
// C2: WASM integrity pinning
// ------------------------------------------------------------------

#[test]
fn c2_manifest_pins_bytes_and_leaves_the_sandbox_closed() {
    let config = PluginsConfig {
        api_version: 1,
        timeout_ms: 1234,
        memory_limit_pages: 77,
        plugin: vec![],
    };
    let bytes = b"verified wasm bytes".to_vec();
    let manifest = build_manifest(bytes.clone(), &config);

    // Regression guard for the pin TOCTOU: the manifest handed to Extism
    // must carry the exact bytes we verified (`Wasm::Data`), never a file
    // path Extism would re-open after our hash check (`Wasm::File`).
    assert_eq!(manifest.wasm.len(), 1);
    match &manifest.wasm[0] {
        extism::Wasm::Data { data, .. } => assert_eq!(data, &bytes),
        other => panic!("expected in-memory Wasm::Data, got {other:?}"),
    }
    assert!(manifest.allowed_hosts.is_none());
    assert!(manifest.allowed_paths.is_none());
    assert_eq!(manifest.timeout_ms, Some(1234));
    assert_eq!(manifest.memory.max_pages, Some(77));
}

#[test]
fn c2_verify_sha256_rejects_wrong_digest() {
    let expected = "b".repeat(64);
    let err = verify_sha256("plugin", b"hello wasm bytes", &expected).unwrap_err();
    match err {
        PluginError::IntegrityMismatch {
            name, expected: e, ..
        } => {
            assert_eq!(name, "plugin");
            assert_eq!(e, expected);
        }
        other => panic!("expected IntegrityMismatch, got {other:?}"),
    }
}

#[test]
fn c2_verify_sha256_normalizes_prefix_whitespace_and_case() {
    let bytes = b"hello wasm bytes";
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hex_encode(&hasher.finalize());

    assert!(verify_sha256("plugin", bytes, &digest).is_ok());
    assert!(verify_sha256("plugin", bytes, &digest.to_uppercase()).is_ok());
    assert!(verify_sha256("plugin", bytes, &format!(" sha256:{digest} ")).is_ok());
}

#[test]
fn c2_verify_sha256_reports_malformed_digests_as_a_format_error() {
    let err = verify_sha256("plugin", b"bytes", "deadbeef").unwrap_err();
    assert!(err.to_string().contains("malformed sha256"));
}

// ------------------------------------------------------------------
// C3: env-var least privilege
// ------------------------------------------------------------------

#[test]
fn c3_filter_env_default_empty_allowlist_yields_nothing() {
    let all = HashMap::from([("API_TOKEN".to_string(), "secret".to_string())]);
    let filtered = filter_env(&[], &all);
    assert!(filtered.is_empty());
}

#[test]
fn c3_filter_env_only_passes_allowlisted_keys() {
    let all = HashMap::from([
        ("API_TOKEN".to_string(), "secret".to_string()),
        ("OTHER_SECRET".to_string(), "nope".to_string()),
    ]);
    let allow = vec!["API_TOKEN".to_string()];
    let filtered = filter_env(&allow, &all);
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered.get("API_TOKEN").map(String::as_str),
        Some("secret")
    );
    assert!(!filtered.contains_key("OTHER_SECRET"));
}

#[test]
fn c3_filter_env_is_case_sensitive() {
    let all = HashMap::from([("api_token".to_string(), "secret".to_string())]);
    let allow = vec!["API_TOKEN".to_string()];
    let filtered = filter_env(&allow, &all);
    assert!(filtered.is_empty());
}

// ------------------------------------------------------------------
// Live WASM
// ------------------------------------------------------------------

#[tokio::test]
async fn a_spinning_plugin_is_stopped_by_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_project_config(
        tmp.path(),
        r#"
api_version = 1
timeout_ms = 200
[[plugin]]
name = "spin"
path = "spin.wasm"
capabilities = ["pre_request"]
"#,
    );
    write_wasm(&dir, "spin.wasm", SPIN_WAT);

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    assert_eq!(registry.len(), 1);

    let err = registry.plugins[0]
        .call("run", String::new(), HashMap::new())
        .await
        .unwrap_err();

    // The `Timeout` match itself proves the call didn't hang forever
    // — a wall-clock `elapsed() < 5s` assertion on top is redundant.
    assert!(
        matches!(
            err,
            PluginError::Timeout {
                timeout_ms: 200,
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn a_trapping_plugin_fails_alone_and_the_next_plugin_still_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_project_config(
        tmp.path(),
        r#"
api_version = 1
[[plugin]]
name = "trap"
path = "trap.wasm"
capabilities = ["pre_request"]

[[plugin]]
name = "echo"
path = "echo.wasm"
capabilities = ["pre_request"]
"#,
    );
    write_wasm(&dir, "trap.wasm", TRAP_WAT);
    write_wasm(&dir, "echo.wasm", ECHO_WAT);

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    assert_eq!(registry.plugin_names(), vec!["trap", "echo"]);

    let mut outcomes = Vec::new();
    for plugin in registry.plugins_with_capability(&PluginCapability::PreRequest) {
        outcomes.push(
            plugin
                .call("run", "payload".to_string(), HashMap::new())
                .await,
        );
    }

    assert!(matches!(
        outcomes[0],
        Err(PluginError::ExecutionFailed { .. })
    ));
    assert_eq!(outcomes[1].as_deref().unwrap(), "payload");
}

#[tokio::test]
async fn a_plugin_only_sees_allowlisted_env_values() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_project_config(
        tmp.path(),
        r#"
api_version = 1
[[plugin]]
name = "reader"
path = "reader.wasm"
capabilities = ["pre_request"]
env_allowlist = ["ALLOWED"]
"#,
    );
    write_wasm(&dir, "reader.wasm", READ_ENV_WAT);

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    let env = HashMap::from([
        ("ALLOWED".to_string(), "visible".to_string()),
        ("DENIED".to_string(), "top-secret".to_string()),
    ]);

    let allowed = registry.plugins[0]
        .call("run", "ALLOWED".to_string(), env.clone())
        .await
        .unwrap();
    assert_eq!(allowed, "visible");

    let denied = registry.plugins[0]
        .call("run", "DENIED".to_string(), env)
        .await
        .unwrap();
    assert!(denied.is_empty(), "withheld value leaked: {denied}");
}

#[tokio::test]
async fn has_function_survives_a_poisoned_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_project_config(
        tmp.path(),
        r#"
api_version = 1
[[plugin]]
name = "echo"
path = "echo.wasm"
capabilities = ["pre_request"]
"#,
    );
    write_wasm(&dir, "echo.wasm", ECHO_WAT);

    let registry = PluginRegistry::discover_and_load_with(tmp.path(), None, true);
    let shared = Arc::clone(&registry.plugins[0].plugin);

    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = shared.lock().unwrap();
        panic!("poison the plugin mutex");
    }));
    std::panic::set_hook(previous_hook);

    assert!(shared.is_poisoned());
    assert!(registry.plugins[0].has_function("run"));
}
