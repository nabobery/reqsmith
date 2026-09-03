use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use extism::{CurrentPlugin, Error, Function, UserData, Val, ValType};

use super::errors::PluginError;

/// Lock host data, recovering from a mutex poisoned by an earlier panicking
/// host call rather than panicking again on the runtime thread. Release
/// builds use `panic = "abort"`, so this recovery path only ever matters in
/// debug/test builds, where a panic unwinds instead of aborting.
fn lock_host_data(data: &Mutex<HostData>) -> MutexGuard<'_, HostData> {
    data.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Data made available to host functions called from plugins.
#[derive(Debug, Clone)]
pub struct HostData {
    /// Environment variable values the plugin may read.
    pub env_values: HashMap<String, String>,
    /// Plugin-specific configuration from `plugins.toml`.
    pub plugin_config: HashMap<String, String>,
}

#[derive(Clone)]
pub struct HostContext {
    data: UserData<HostData>,
}

impl HostContext {
    pub fn new(data: HostData) -> Self {
        Self {
            data: UserData::new(data),
        }
    }

    pub fn build_functions(&self) -> Vec<Function> {
        vec![
            Function::new(
                "provide_env_var",
                [ValType::I64],
                [ValType::I64],
                self.data.clone(),
                provide_env_var,
            ),
            Function::new(
                "plugin_log",
                [ValType::I64],
                [ValType::I64],
                self.data.clone(),
                plugin_log,
            ),
            Function::new(
                "read_config",
                [ValType::I64],
                [ValType::I64],
                self.data.clone(),
                read_config,
            ),
        ]
    }

    /// Replace the env values visible to this plugin's `provide_env_var`.
    pub fn set_env_values(&self, env_values: HashMap<String, String>) -> Result<(), PluginError> {
        let data = self
            .data
            .get()
            .map_err(|e| PluginError::HostDataUnavailable(e.to_string()))?;
        lock_host_data(&data).env_values = env_values;
        Ok(())
    }
}

/// Host function: plugins call this to read an environment variable.
fn provide_env_var(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    data: UserData<HostData>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let shared = data.get()?;
    let value = lock_host_data(&shared)
        .env_values
        .get(&key)
        .cloned()
        .unwrap_or_default();
    let handle = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

/// Host function: plugins call this to emit a log message.
fn plugin_log(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _data: UserData<HostData>,
) -> Result<(), Error> {
    let msg: String = plugin.memory_get_val(&inputs[0])?;
    tracing::info!(source = "plugin", "{msg}");
    let handle = plugin.memory_new("")?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

/// Host function: plugins call this to read their configuration.
fn read_config(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    data: UserData<HostData>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let shared = data.get()?;
    let value = lock_host_data(&shared)
        .plugin_config
        .get(&key)
        .cloned()
        .unwrap_or_default();
    let handle = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HostContext, HostData};
    use std::collections::HashMap;

    #[test]
    fn set_env_values_replaces_runtime_environment() {
        let host_context = HostContext::new(HostData {
            env_values: HashMap::from([("OLD".into(), "value".into())]),
            plugin_config: HashMap::from([("region".into(), "us-east-1".into())]),
        });

        host_context
            .set_env_values(HashMap::from([("API_TOKEN".into(), "secret".into())]))
            .unwrap();

        let shared = host_context.data.get().unwrap();
        let data = super::lock_host_data(&shared);
        assert_eq!(
            data.env_values.get("API_TOKEN").map(String::as_str),
            Some("secret")
        );
        assert!(!data.env_values.contains_key("OLD"));
        assert_eq!(
            data.plugin_config.get("region").map(String::as_str),
            Some("us-east-1")
        );
    }
}
