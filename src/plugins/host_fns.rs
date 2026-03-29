use std::collections::HashMap;

use extism::{CurrentPlugin, Error, Function, UserData, Val, ValType};

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

    pub fn set_env_values(&self, env_values: HashMap<String, String>) {
        if let Ok(mut data) = self.data.get().expect("host data should exist").lock() {
            data.env_values = env_values;
        }
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
    let value = data
        .get()?
        .lock()
        .unwrap()
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
    let value = data
        .get()?
        .lock()
        .unwrap()
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

        host_context.set_env_values(HashMap::from([("API_TOKEN".into(), "secret".into())]));

        let shared = host_context.data.get().expect("host data should exist");
        let data = shared.lock().unwrap();
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
