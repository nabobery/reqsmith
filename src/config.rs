use std::time::Duration;

use crate::cli::Cli;

pub struct Config {
    pub tick_rate: Duration,
    pub frame_rate: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(250),
            frame_rate: Duration::from_millis(16), // ~60 FPS
        }
    }
}

impl Config {
    pub fn from_cli(cli: &Cli) -> Self {
        let mut config = Self::default();
        if let Some(tick) = cli.tick_rate {
            config.tick_rate = Duration::from_millis(tick);
        }
        if let Some(frame) = cli.frame_rate {
            config.frame_rate = Duration::from_millis(frame);
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_reasonable_tick_rate() {
        let config = Config::default();
        assert_eq!(config.tick_rate, Duration::from_millis(250));
        assert_eq!(config.frame_rate, Duration::from_millis(16));
    }

    #[test]
    fn config_from_cli_overrides_tick_rate() {
        let cli = Cli {
            debug: false,
            tick_rate: Some(100),
            frame_rate: Some(33),
        };
        let config = Config::from_cli(&cli);
        assert_eq!(config.tick_rate, Duration::from_millis(100));
        assert_eq!(config.frame_rate, Duration::from_millis(33));
    }

    #[test]
    fn config_from_cli_preserves_defaults_when_none() {
        let cli = Cli {
            debug: false,
            tick_rate: None,
            frame_rate: None,
        };
        let config = Config::from_cli(&cli);
        assert_eq!(config.tick_rate, Duration::from_millis(250));
        assert_eq!(config.frame_rate, Duration::from_millis(16));
    }
}
