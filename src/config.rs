use std::time::Duration;

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
    pub fn new(tick_rate: Option<u64>, frame_rate: Option<u64>) -> Self {
        let mut config = Self::default();
        if let Some(tick) = tick_rate {
            config.tick_rate = Duration::from_millis(tick);
        }
        if let Some(frame) = frame_rate {
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
    fn config_overrides_tick_rate() {
        let config = Config::new(Some(100), Some(33));
        assert_eq!(config.tick_rate, Duration::from_millis(100));
        assert_eq!(config.frame_rate, Duration::from_millis(33));
    }

    #[test]
    fn config_preserves_defaults_when_none() {
        let config = Config::new(None, None);
        assert_eq!(config.tick_rate, Duration::from_millis(250));
        assert_eq!(config.frame_rate, Duration::from_millis(16));
    }
}
