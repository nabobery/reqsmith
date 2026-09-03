use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::core::redaction::RedactionPolicy;
use crate::plugins::config::{
    ALLOW_PROJECT_PLUGINS_VAR, DiscoveredConfig, PluginEntry, discover_configs,
    looks_like_secret_name,
};
use crate::plugins::errors::PluginError;
use crate::plugins::registry::PluginRegistry;
use crate::plugins::registry::resolve_wasm_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginListStatus {
    Loaded,
    Disabled,
    MissingWasm,
    LoadFailed,
    BlockedUntrusted,
}

impl PluginListStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Disabled => "disabled",
            Self::MissingWasm => "missing_wasm",
            Self::LoadFailed => "load_failed",
            Self::BlockedUntrusted => "blocked_untrusted",
        }
    }
}

/// What registry loading reported, flattened so rendering is testable without
/// instantiating any WASM.
#[derive(Debug, Default)]
struct RegistryView {
    loaded: HashSet<(PathBuf, String)>,
    errors: HashMap<(PathBuf, String), String>,
    blocked_project_config: Option<PathBuf>,
}

impl RegistryView {
    fn from_registry(registry: &PluginRegistry) -> Self {
        Self {
            loaded: registry
                .loaded_plugin_ids()
                .map(|(path, name)| (path.to_path_buf(), name.to_owned()))
                .collect(),
            errors: registry
                .load_errors()
                .iter()
                .map(|failure| {
                    (
                        (failure.config_path.clone(), failure.name.clone()),
                        failure.error.to_string(),
                    )
                })
                .collect(),
            blocked_project_config: registry.blocked_project_config().map(Path::to_path_buf),
        }
    }
}

fn plugin_status(
    entry: &PluginEntry,
    discovered: &DiscoveredConfig,
    cwd: &Path,
    view: &RegistryView,
) -> PluginListStatus {
    if view.blocked_project_config.as_deref() == Some(discovered.path.as_path()) {
        return PluginListStatus::BlockedUntrusted;
    }
    if !entry.enabled {
        return PluginListStatus::Disabled;
    }
    let id = (discovered.path.clone(), entry.name.clone());
    if view.errors.contains_key(&id) {
        return PluginListStatus::LoadFailed;
    }
    match resolve_wasm_path(discovered, cwd, &entry.path) {
        Err(_) => return PluginListStatus::LoadFailed,
        Ok(path) if !path.is_file() => return PluginListStatus::MissingWasm,
        Ok(_) => {}
    }

    if view.loaded.contains(&id) {
        PluginListStatus::Loaded
    } else {
        PluginListStatus::LoadFailed
    }
}

fn capability_label(capability: &crate::plugins::models::PluginCapability) -> &'static str {
    use crate::plugins::models::PluginCapability::*;
    match capability {
        PreRequest => "pre_request",
        PostResponse => "post_response",
        Authenticate => "authenticate",
        ProvideVariable => "provide_variable",
    }
}

fn render_plugin_list(
    configs: &[DiscoveredConfig],
    cwd: &Path,
    view: &RegistryView,
    config_errors: &[(PathBuf, PluginError)],
) -> String {
    let mut output = String::new();
    let caps_header = "CAPABILITIES";
    let _ = writeln!(
        output,
        "{:<20} {:<12} {:<18} {caps_header}",
        "NAME", "ENABLED", "STATUS"
    );
    let _ = writeln!(output, "{}", "-".repeat(80));

    for discovered in configs {
        let _ = writeln!(output, "# {}", discovered.path.display());
        for entry in &discovered.config.plugin {
            let caps: Vec<&str> = entry.capabilities.iter().map(capability_label).collect();
            let status = plugin_status(entry, discovered, cwd, view);
            let _ = writeln!(
                output,
                "{:<20} {:<12} {:<18} {}",
                entry.name,
                if entry.enabled { "yes" } else { "no" },
                status.label(),
                caps.join(", ")
            );
        }
    }

    if let Some(blocked) = &view.blocked_project_config {
        let _ = writeln!(
            output,
            "\nProject-local plugin config not loaded: {}\nSet {ALLOW_PROJECT_PLUGINS_VAR}=1 to run \
             plugins shipped inside this project, and only for repositories you trust.",
            blocked.display()
        );
    }

    // A config that failed to read/parse/validate must not hide the
    // configs that DID parse — surface it as a warning line instead.
    for (path, error) in config_errors {
        let _ = writeln!(
            output,
            "\nWarning: plugin config {} failed to load: {error}",
            path.display()
        );
    }

    output
}

/// Execute `reqsmith plugin list`.
pub fn execute_list() -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = discover_configs(&cwd);

    // Discovery is per-config fault-tolerant, so a config that
    // failed to parse never hides one that did. Only bail out (non-zero
    // exit) when NOTHING parsed at all; otherwise render what we have and
    // warn about the rest.
    if discovered.configs.is_empty() {
        for (path, error) in &discovered.errors {
            eprintln!(
                "Warning: plugin config {} failed to load: {error}",
                path.display()
            );
        }
        if discovered.errors.is_empty() {
            println!("No plugins found.");
            println!();
            println!("To add plugins, create .reqsmith/plugins.toml in your project.");
            return ExitCode::SUCCESS;
        }
        return ExitCode::from(1);
    }

    let registry = PluginRegistry::discover_and_load(&cwd);
    let view = RegistryView::from_registry(&registry);

    print!(
        "{}",
        render_plugin_list(&discovered.configs, &cwd, &view, &discovered.errors)
    );

    for ((path, name), error) in &view.errors {
        eprintln!(
            "Warning: plugin '{name}' from {} failed to load: {error}",
            path.display()
        );
    }

    ExitCode::SUCCESS
}

fn render_plugin_info(
    entry: &PluginEntry,
    discovered: &DiscoveredConfig,
    cwd: &Path,
    view: &RegistryView,
    policy: &RedactionPolicy,
) -> String {
    let mut output = String::new();
    let status = plugin_status(entry, discovered, cwd, view);
    let resolved = resolve_wasm_path(discovered, cwd, &entry.path);

    let _ = writeln!(output, "Name:          {}", entry.name);
    match &resolved {
        Ok(path) => {
            let _ = writeln!(output, "Path:          {}", path.display());
        }
        Err(e) => {
            let _ = writeln!(output, "Path:          <rejected> {e}");
        }
    }
    let _ = writeln!(output, "Config file:   {}", discovered.path.display());
    let _ = writeln!(output, "Enabled:       {}", entry.enabled);
    let _ = writeln!(output, "Status:        {}", status.label());
    let _ = writeln!(
        output,
        "Capabilities:  {}",
        entry
            .capabilities
            .iter()
            .map(capability_label)
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(
        output,
        "SHA-256:       {}",
        entry.sha256.as_deref().unwrap_or("not pinned")
    );
    let allowlist = if entry.env_allowlist.is_empty() {
        "(empty — no environment access)".to_string()
    } else {
        entry
            .env_allowlist
            .iter()
            .map(|name| {
                if looks_like_secret_name(name, policy) {
                    format!("{name} (sensitive)")
                } else {
                    name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(output, "Env allowlist: {allowlist}");

    if !entry.config.is_empty() {
        let _ = writeln!(output, "Config:");
        // A `[plugin.config]` table routinely holds credentials; print it
        // through the same policy that guards response headers.
        for (key, value) in &entry.config {
            let rendered = match value {
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let _ = writeln!(
                output,
                "  {key} = {}",
                policy.redact_value_for(key, &rendered)
            );
        }
    }

    if let Ok(path) = &resolved
        && let Ok(meta) = std::fs::metadata(path)
    {
        let _ = writeln!(output, "WASM size:     {} KB", meta.len() / 1024);
    }
    if let Some(error) = view
        .errors
        .get(&(discovered.path.clone(), entry.name.clone()))
    {
        let _ = writeln!(output, "Load error:    {error}");
    }
    if status == PluginListStatus::BlockedUntrusted {
        let _ = writeln!(
            output,
            "Note:          project-local config not loaded; set \
             {ALLOW_PROJECT_PLUGINS_VAR}=1 to run it, and only for repositories you trust."
        );
    }

    output
}

fn find_plugin<'a>(
    configs: &'a [DiscoveredConfig],
    name: &str,
    preferred_path: Option<&Path>,
) -> Option<(&'a DiscoveredConfig, &'a PluginEntry)> {
    configs.iter().find_map(|discovered| {
        if preferred_path.is_some_and(|path| path != discovered.path) {
            return None;
        }
        discovered
            .config
            .plugin
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| (discovered, entry))
    })
}

/// Execute `reqsmith plugin info <name>`.
pub fn execute_info(name: String) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = discover_configs(&cwd);
    // Warn about any config that failed to parse, but still search
    // whatever configs DID parse rather than bailing out entirely.
    for (path, error) in &discovered.errors {
        eprintln!(
            "Warning: plugin config {} failed to load: {error}",
            path.display()
        );
    }
    let configs = discovered.configs;
    if configs.is_empty() {
        eprintln!("No plugins found.");
        return ExitCode::from(1);
    }

    let registry = PluginRegistry::discover_and_load(&cwd);
    let view = RegistryView::from_registry(&registry);
    let policy = RedactionPolicy::from_env();

    let found = find_plugin(&configs, &name, registry.plugin_config_path(&name))
        .or_else(|| find_plugin(&configs, &name, None));

    match found {
        Some((discovered, entry)) => {
            print!(
                "{}",
                render_plugin_info(entry, discovered, &cwd, &view, &policy)
            );
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("Plugin '{name}' not found.");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::plugins::config::{ConfigSource, PluginsConfig};
    use crate::plugins::models::PluginCapability;

    use super::*;

    fn discovered_config(root: &Path) -> DiscoveredConfig {
        DiscoveredConfig {
            path: root.join(".reqsmith/plugins.toml"),
            config: PluginsConfig {
                api_version: 1,
                timeout_ms: 5000,
                memory_limit_pages: 256,
                plugin: vec![],
            },
            source: ConfigSource::ProjectLocal,
        }
    }

    fn plugin_entry(name: &str, path: &str, enabled: bool) -> PluginEntry {
        PluginEntry {
            name: name.into(),
            path: PathBuf::from(path),
            enabled,
            capabilities: vec![PluginCapability::PreRequest],
            config: HashMap::new(),
            sha256: None,
            env_allowlist: vec![],
        }
    }

    fn view_with_loaded(config_path: &Path, names: &[&str]) -> RegistryView {
        RegistryView {
            loaded: names
                .iter()
                .map(|name| (config_path.to_path_buf(), (*name).to_owned()))
                .collect(),
            ..RegistryView::default()
        }
    }

    #[test]
    fn plugin_status_reports_disabled_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("disabled", "plugins/test.wasm", false);

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), &RegistryView::default()),
            PluginListStatus::Disabled
        );
    }

    #[test]
    fn plugin_status_reports_missing_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("missing", "plugins/test.wasm", true);

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), &RegistryView::default()),
            PluginListStatus::MissingWasm
        );
    }

    #[test]
    fn plugin_status_reports_loaded_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins_dir = tmp.path().join(".reqsmith/plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        std::fs::write(plugins_dir.join("loaded.wasm"), b"wasm").unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("loaded", "plugins/loaded.wasm", true);

        assert_eq!(
            plugin_status(
                &entry,
                &discovered,
                tmp.path(),
                &view_with_loaded(&discovered.path, &["loaded"])
            ),
            PluginListStatus::Loaded
        );
    }

    #[test]
    fn duplicate_error_does_not_taint_the_loaded_definition() {
        let tmp = tempfile::tempdir().unwrap();
        let mut global = discovered_config(tmp.path());
        global.path = tmp.path().join("global/plugins.toml");
        let mut project = discovered_config(tmp.path());
        project.path = tmp.path().join("project/plugins.toml");
        let entry = plugin_entry("dup", "dup.wasm", true);
        for config in [&global, &project] {
            std::fs::create_dir_all(config.path.parent().unwrap()).unwrap();
            std::fs::write(config.path.parent().unwrap().join("dup.wasm"), b"wasm").unwrap();
        }
        let view = RegistryView {
            loaded: HashSet::from([(global.path.clone(), "dup".to_string())]),
            errors: HashMap::from([(
                (project.path.clone(), "dup".to_string()),
                "duplicate".to_string(),
            )]),
            blocked_project_config: None,
        };

        assert_eq!(
            plugin_status(&entry, &global, tmp.path(), &view),
            PluginListStatus::Loaded
        );
        assert_eq!(
            plugin_status(&entry, &project, tmp.path(), &view),
            PluginListStatus::LoadFailed
        );
    }

    #[test]
    fn plugin_info_selection_prefers_the_active_config() {
        let tmp = tempfile::tempdir().unwrap();
        let mut project = discovered_config(tmp.path());
        project.path = tmp.path().join("project/plugins.toml");
        project.config.plugin = vec![plugin_entry("dup", "project.wasm", true)];
        let mut global = discovered_config(tmp.path());
        global.path = tmp.path().join("global/plugins.toml");
        global.config.plugin = vec![plugin_entry("dup", "global.wasm", true)];
        let configs = vec![project, global];

        let (selected, entry) =
            find_plugin(&configs, "dup", Some(&configs[1].path)).expect("plugin exists");

        assert_eq!(selected.path, configs[1].path);
        assert_eq!(entry.path, PathBuf::from("global.wasm"));
    }

    #[test]
    fn plugin_status_reports_per_entry_load_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("broken", "plugins/broken.wasm", true);
        let view = RegistryView {
            loaded: HashSet::new(),
            errors: HashMap::from([(
                (discovered.path.clone(), "broken".to_string()),
                "integrity mismatch".to_string(),
            )]),
            blocked_project_config: None,
        };

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), &view),
            PluginListStatus::LoadFailed
        );
    }

    #[test]
    fn plugin_status_reports_blocked_project_local_config() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("untrusted", "plugins/untrusted.wasm", true);
        let view = RegistryView {
            loaded: HashSet::new(),
            errors: HashMap::new(),
            blocked_project_config: Some(discovered.path.clone()),
        };

        assert_eq!(
            plugin_status(&entry, &discovered, tmp.path(), &view),
            PluginListStatus::BlockedUntrusted
        );
    }

    #[test]
    fn render_plugin_list_includes_status_column() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins_dir = tmp.path().join(".reqsmith/plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        std::fs::write(plugins_dir.join("loaded.wasm"), b"wasm").unwrap();

        let mut discovered = discovered_config(tmp.path());
        discovered.config.plugin = vec![
            plugin_entry("loaded", "plugins/loaded.wasm", true),
            plugin_entry("disabled", "plugins/missing.wasm", false),
        ];

        let rendered = render_plugin_list(
            std::slice::from_ref(&discovered),
            tmp.path(),
            &view_with_loaded(&discovered.path, &["loaded"]),
            &[],
        );

        assert!(rendered.contains("STATUS"));
        assert!(rendered.contains("loaded"));
        assert!(rendered.contains("disabled"));
    }

    #[test]
    fn render_plugin_list_flags_blocked_entries_and_names_the_opt_in() {
        let tmp = tempfile::tempdir().unwrap();
        let mut discovered = discovered_config(tmp.path());
        discovered.config.plugin = vec![plugin_entry("untrusted", "untrusted.wasm", true)];
        let view = RegistryView {
            loaded: HashSet::new(),
            errors: HashMap::new(),
            blocked_project_config: Some(discovered.path.clone()),
        };

        let rendered =
            render_plugin_list(std::slice::from_ref(&discovered), tmp.path(), &view, &[]);

        assert!(rendered.contains("blocked_untrusted"), "{rendered}");
        assert!(rendered.contains(ALLOW_PROJECT_PLUGINS_VAR), "{rendered}");
    }

    // ------------------------------------------------------------------
    // per-config discovery fault tolerance
    // ------------------------------------------------------------------

    #[test]
    fn render_plugin_list_includes_global_entries_and_a_warning_for_the_invalid_config() {
        let tmp = tempfile::tempdir().unwrap();
        // Only the config that DID parse (e.g. the user-global one) is ever
        // passed in `configs` — the invalid project-local one never makes it
        // into `DiscoveredConfigs::configs`, only into `errors`.
        let mut global = discovered_config(tmp.path());
        global.source = ConfigSource::UserGlobal;
        global.config.plugin = vec![plugin_entry("from-global", "g.wasm", true)];

        let bad_project_path = tmp.path().join(".reqsmith/plugins.toml");
        let config_errors = vec![(
            bad_project_path.clone(),
            crate::plugins::errors::PluginError::ManifestError("bad toml".into()),
        )];

        let rendered = render_plugin_list(
            std::slice::from_ref(&global),
            tmp.path(),
            &RegistryView::default(),
            &config_errors,
        );

        assert!(rendered.contains("from-global"), "{rendered}");
        assert!(
            rendered.contains(&bad_project_path.display().to_string()),
            "{rendered}"
        );
        assert!(rendered.contains("failed to load"), "{rendered}");
    }

    #[test]
    fn render_plugin_info_shows_pinning_and_allowlist_sensitivity() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let mut entry = plugin_entry("auth", "auth.wasm", true);
        entry.sha256 = Some(format!("sha256:{}", "a".repeat(64)));
        // `RedactionPolicy::is_sensitive` alone (exact HTTP header names)
        // misses env-var-style secret names like these; the fix routes this
        // through `looks_like_secret_name` instead.
        entry.env_allowlist = vec![
            "REGION".into(),
            "AWS_SECRET_ACCESS_KEY".into(),
            "GITHUB_TOKEN".into(),
        ];

        let rendered = render_plugin_info(
            &entry,
            &discovered,
            tmp.path(),
            &RegistryView::default(),
            &RedactionPolicy::default(),
        );

        assert!(rendered.contains(&"a".repeat(64)), "{rendered}");
        assert!(
            rendered.contains("AWS_SECRET_ACCESS_KEY (sensitive)"),
            "{rendered}"
        );
        assert!(rendered.contains("GITHUB_TOKEN (sensitive)"), "{rendered}");
        assert!(!rendered.contains("REGION (sensitive)"), "{rendered}");
    }

    #[test]
    fn render_plugin_info_shows_the_error_for_the_rejected_definition() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("dup", "dup.wasm", true);
        let view = RegistryView {
            loaded: HashSet::new(),
            errors: HashMap::from([(
                (discovered.path.clone(), "dup".to_string()),
                "plugin 'dup' is already defined in /other/plugins.toml".to_string(),
            )]),
            blocked_project_config: None,
        };

        let rendered = render_plugin_info(
            &entry,
            &discovered,
            tmp.path(),
            &view,
            &RedactionPolicy::default(),
        );

        assert!(rendered.contains("Load error:"), "{rendered}");
        assert!(rendered.contains("already defined"), "{rendered}");
    }

    #[test]
    fn render_plugin_info_reports_unpinned_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let entry = plugin_entry("plain", "plain.wasm", true);

        let rendered = render_plugin_info(
            &entry,
            &discovered,
            tmp.path(),
            &RegistryView::default(),
            &RedactionPolicy::default(),
        );

        assert!(rendered.contains("not pinned"), "{rendered}");
        assert!(rendered.contains("(empty"), "{rendered}");
    }

    #[test]
    fn render_plugin_info_redacts_sensitive_config_values() {
        let tmp = tempfile::tempdir().unwrap();
        let discovered = discovered_config(tmp.path());
        let mut entry = plugin_entry("auth", "auth.wasm", true);
        entry.config = HashMap::from([
            ("api-key".into(), toml::Value::String("s3cr3t".into())),
            ("region".into(), toml::Value::String("us-east-1".into())),
        ]);

        let rendered = render_plugin_info(
            &entry,
            &discovered,
            tmp.path(),
            &RegistryView::default(),
            &RedactionPolicy::default(),
        );

        assert!(!rendered.contains("s3cr3t"), "{rendered}");
        assert!(rendered.contains("us-east-1"), "{rendered}");
    }
}
