//! Persistent configuration, stored as JSON in the per-user config directory
//! (`%APPDATA%\PingAgent\config.json` on Windows).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::schedule::{self, Day};

pub const APP_NAME: &str = "PingAgent";
pub const DEFAULT_HOST: &str = "192.168.86.1";

pub const MIN_INTERVAL_SECS: u64 = 1;
pub const MAX_INTERVAL_SECS: u64 = 3600;
pub const MIN_TIMEOUT_MS: u64 = 100;
pub const MAX_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Host name or IP address to ping.
    pub host: String,
    /// Seconds between pings.
    pub interval_secs: u64,
    /// How long to wait for a reply before counting the ping as lost.
    pub timeout_ms: u64,
    /// Round-trip times at or above this are shown in the "warn" colour.
    pub warn_ms: u64,
    /// Round-trip times at or above this are shown in the "bad" colour.
    pub bad_ms: u64,
    /// When true, `interval_secs` applies only inside the active window on
    /// `active_days`; `idle_interval_secs` applies the rest of the time.
    pub schedule_enabled: bool,
    /// Start of the active window, local time, "HH:MM" (24-hour).
    pub active_start: String,
    /// End of the active window, local time, "HH:MM". Earlier than the start
    /// means the window runs past midnight.
    pub active_end: String,
    /// Days on which the active window applies.
    pub active_days: Vec<Day>,
    /// Seconds between pings outside the active window.
    pub idle_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: DEFAULT_HOST.to_string(),
            interval_secs: 5,
            timeout_ms: 1000,
            warn_ms: 50,
            bad_ms: 150,
            schedule_enabled: false,
            active_start: "08:00".to_string(),
            active_end: "18:00".to_string(),
            active_days: Day::WEEKDAYS.to_vec(),
            idle_interval_secs: 60,
        }
    }
}

impl Config {
    /// Location of the config file. `None` if the platform has no config directory.
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_NAME).join("config.json"))
    }

    /// Load the config from disk. Missing or unreadable files yield the defaults;
    /// a present-but-invalid file is reported as an error so the user can be told.
    pub fn load() -> Result<Config, String> {
        let Some(path) = Self::path() else {
            return Ok(Config::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                Self::from_json(&text).map_err(|e| format!("{} is not valid: {e}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::path() else {
            return Err("no config directory available on this system".to_string());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
    }

    pub fn from_json(text: &str) -> Result<Config, String> {
        let cfg: Config = serde_json::from_str(text).map_err(|e| e.to_string())?;
        cfg.validate()?;
        Ok(cfg.normalized())
    }

    /// Check that the values make sense. Returns a human-readable message otherwise.
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Host must not be empty.".into());
        }
        if self.host.trim().chars().any(char::is_whitespace) {
            return Err("Host must not contain spaces.".into());
        }
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&self.interval_secs) {
            return Err(format!(
                "Interval must be between {MIN_INTERVAL_SECS} and {MAX_INTERVAL_SECS} seconds."
            ));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "Timeout must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS} ms."
            ));
        }
        if self.warn_ms == 0 || self.bad_ms == 0 {
            return Err("Colour thresholds must be greater than zero.".into());
        }
        if self.warn_ms > self.bad_ms {
            return Err("The amber threshold must not be higher than the red threshold.".into());
        }
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&self.idle_interval_secs) {
            return Err(format!(
                "The outside-hours interval must be between {MIN_INTERVAL_SECS} and {MAX_INTERVAL_SECS} seconds."
            ));
        }
        schedule::parse_hhmm(&self.active_start).map_err(|e| format!("Active hours start: {e}"))?;
        schedule::parse_hhmm(&self.active_end).map_err(|e| format!("Active hours end: {e}"))?;
        if self.schedule_enabled && self.active_days.is_empty() {
            return Err("Pick at least one day for the schedule.".into());
        }
        Ok(())
    }

    /// Trim text fields and store times in canonical "HH:MM" form.
    pub fn normalized(mut self) -> Config {
        self.host = self.host.trim().to_string();
        if let Ok(m) = schedule::parse_hhmm(&self.active_start) {
            self.active_start = schedule::format_hhmm(m);
        }
        if let Ok(m) = schedule::parse_hhmm(&self.active_end) {
            self.active_end = schedule::format_hhmm(m);
        }
        self.active_days.dedup();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let cfg = Config::default();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.host, "192.168.86.1");
        assert_eq!(cfg.interval_secs, 5);
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = Config {
            host: "example.com".into(),
            interval_secs: 10,
            ..Config::default()
        };
        let text = serde_json::to_string(&cfg).unwrap();
        assert_eq!(Config::from_json(&text).unwrap(), cfg);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg = Config::from_json(r#"{"host": "  10.0.0.1 "}"#).unwrap();
        assert_eq!(cfg.host, "10.0.0.1");
        assert_eq!(cfg.timeout_ms, Config::default().timeout_ms);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::from_json(r#"{"host": ""}"#).is_err());
        assert!(Config::from_json(r#"{"interval_secs": 0}"#).is_err());
        assert!(Config::from_json(r#"{"warn_ms": 500, "bad_ms": 100}"#).is_err());
        assert!(Config::from_json(r#"{"active_start": "8am"}"#).is_err());
        assert!(Config::from_json(r#"{"idle_interval_secs": 0}"#).is_err());
        assert!(Config::from_json(r#"{"schedule_enabled": true, "active_days": []}"#).is_err());
        assert!(Config::from_json(r#"{"active_days": ["Monday"]}"#).is_err());
        assert!(Config::from_json("not json").is_err());
    }

    #[test]
    fn old_config_without_schedule_fields_still_loads() {
        let cfg = Config::from_json(
            r#"{"host":"10.0.0.1","interval_secs":5,"timeout_ms":1000,"warn_ms":50,"bad_ms":150}"#,
        )
        .unwrap();
        assert!(!cfg.schedule_enabled);
        assert_eq!(cfg.idle_interval_secs, 60);
        assert_eq!(cfg.active_days, Day::WEEKDAYS.to_vec());
    }

    #[test]
    fn times_are_canonicalised() {
        let cfg = Config::from_json(r#"{"active_start": "8:5", "active_end": " 17:30 "}"#).unwrap();
        assert_eq!(cfg.active_start, "08:05");
        assert_eq!(cfg.active_end, "17:30");
    }
}
