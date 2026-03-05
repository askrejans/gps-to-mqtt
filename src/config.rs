//! Configuration management for gps-to-mqtt.
//!
//! Loads settings from TOML files (searched in standard locations) and from
//! `GPS_TO_MQTT_*` environment variables (highest priority).
//!
//! Priority (highest → lowest):
//! 1. `GPS_TO_MQTT_*` environment variables
//! 2. Specified config file (`--config`)
//! 3. Default search paths (`/etc/gps-to-mqtt/`, exe dir, `./`)
//! 4. Built-in defaults

use anyhow::Result;
use config::{Config, Environment, File};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const VALID_BAUD_RATES: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

/// Application configuration — all fields have serde defaults so an empty
/// (or partially filled) TOML file works without errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // --- Serial port ---
    /// Serial device path, e.g. `/dev/ttyUSB0` or `COM3`
    #[serde(default = "default_port_name")]
    pub port_name: String,

    /// Baud rate for the GPS receiver
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,

    /// Send a UBX command to switch the receiver to 10 Hz
    #[serde(default)]
    pub set_gps_to_10hz: bool,

    // --- MQTT ---
    /// Enable MQTT publishing (`false` = display-only / TUI mode)
    #[serde(default = "default_mqtt_enabled")]
    pub mqtt_enabled: bool,

    /// MQTT broker hostname or IP
    #[serde(default = "default_mqtt_host")]
    pub mqtt_host: String,

    /// MQTT broker port
    #[serde(default = "default_mqtt_port")]
    pub mqtt_port: u16,

    /// MQTT client ID — auto-generated when `None`
    pub mqtt_client_id: Option<String>,

    /// Base MQTT topic prefix, e.g. `/GOLF86/GPS`
    #[serde(default = "default_mqtt_base_topic")]
    pub mqtt_base_topic: String,

    /// Maximum reconnection attempts; `0` = infinite
    #[serde(default)]
    pub mqtt_reconnect_max_attempts: u32,

    /// MQTT broker username (optional)
    pub mqtt_username: Option<String>,

    /// MQTT broker password (optional)
    pub mqtt_password: Option<String>,

    /// Enable TLS for the MQTT connection
    #[serde(default)]
    pub mqtt_use_tls: bool,

    // --- Logging ---
    /// Log level: `trace` | `debug` | `info` | `warn` | `error`
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Emit JSON-structured logs (useful for log aggregation)
    #[serde(default)]
    pub log_json: bool,

    // --- TUI ---
    /// TUI render interval in milliseconds
    #[serde(default = "default_tui_refresh_rate_ms")]
    pub tui_refresh_rate_ms: u64,

    /// Maximum lines kept in the TUI log ring-buffer
    #[serde(default = "default_max_log_buffer_size")]
    pub max_log_buffer_size: usize,

    // --- Telemetry ---
    /// Enable telemetry calculations (acceleration, g-forces, distance …)
    #[serde(default = "default_telemetry_enabled")]
    pub telemetry_enabled: bool,

    /// Moving-average window for telemetry smoothing
    #[serde(default = "default_telemetry_smoothing_window")]
    pub telemetry_smoothing_window: usize,

    // --- Track / lap timing ---
    /// Lap-timing mode: `disabled` | `manual` | `learn` | `gpx`
    #[serde(default = "default_track_mode")]
    pub track_mode: String,

    /// Start/Finish latitude (required for `manual` mode)
    pub track_start_lat: Option<f64>,

    /// Start/Finish longitude (required for `manual` mode)
    pub track_start_lon: Option<f64>,

    /// Geofence radius around start/finish in metres
    #[serde(default = "default_track_geofence_radius")]
    pub track_geofence_radius: f64,

    /// Path to GPX file (required for `gpx` mode)
    pub track_gpx_file: Option<String>,

    /// Internal: path to the config file that was loaded (set at runtime)
    #[serde(skip)]
    pub config_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Default value functions
// ---------------------------------------------------------------------------
fn default_port_name() -> String {
    "/dev/ttyUSB0".to_string()
}
fn default_baud_rate() -> u32 {
    9600
}
fn default_mqtt_enabled() -> bool {
    true
}
fn default_mqtt_host() -> String {
    "localhost".to_string()
}
fn default_mqtt_port() -> u16 {
    1883
}
fn default_mqtt_base_topic() -> String {
    "/GOLF86/GPS".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_tui_refresh_rate_ms() -> u64 {
    100
}
fn default_max_log_buffer_size() -> usize {
    1000
}
fn default_telemetry_enabled() -> bool {
    true
}
fn default_telemetry_smoothing_window() -> usize {
    3
}
fn default_track_mode() -> String {
    "disabled".to_string()
}
fn default_track_geofence_radius() -> f64 {
    15.0
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port_name: default_port_name(),
            baud_rate: default_baud_rate(),
            set_gps_to_10hz: false,
            mqtt_enabled: default_mqtt_enabled(),
            mqtt_host: default_mqtt_host(),
            mqtt_port: default_mqtt_port(),
            mqtt_client_id: None,
            mqtt_base_topic: default_mqtt_base_topic(),
            mqtt_reconnect_max_attempts: 0,
            mqtt_username: None,
            mqtt_password: None,
            mqtt_use_tls: false,
            log_level: default_log_level(),
            log_json: false,
            tui_refresh_rate_ms: default_tui_refresh_rate_ms(),
            max_log_buffer_size: default_max_log_buffer_size(),
            telemetry_enabled: default_telemetry_enabled(),
            telemetry_smoothing_window: default_telemetry_smoothing_window(),
            track_mode: default_track_mode(),
            track_start_lat: None,
            track_start_lon: None,
            track_geofence_radius: default_track_geofence_radius(),
            track_gpx_file: None,
            config_path: None,
        }
    }
}

impl AppConfig {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<()> {
        if self.port_name.is_empty() {
            anyhow::bail!("port_name must not be empty");
        }
        if !VALID_BAUD_RATES.contains(&self.baud_rate) {
            anyhow::bail!("baud_rate must be one of: {:?}", VALID_BAUD_RATES);
        }
        if self.mqtt_enabled {
            if self.mqtt_host.is_empty() {
                anyhow::bail!("mqtt_host must not be empty when mqtt_enabled = true");
            }
            if self.mqtt_port == 0 {
                anyhow::bail!("mqtt_port must be > 0");
            }
            if self.mqtt_base_topic.is_empty() {
                anyhow::bail!("mqtt_base_topic must not be empty");
            }
        }
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.log_level.to_lowercase().as_str()) {
            anyhow::bail!("log_level must be one of: {:?}", valid_levels);
        }
        let valid_track_modes = ["disabled", "manual", "learn", "gpx"];
        if !valid_track_modes.contains(&self.track_mode.to_lowercase().as_str()) {
            anyhow::bail!("track_mode must be one of: {:?}", valid_track_modes);
        }
        if self.track_mode == "manual"
            && (self.track_start_lat.is_none() || self.track_start_lon.is_none())
        {
            anyhow::bail!("track_start_lat and track_start_lon are required for manual track_mode");
        }
        if self.track_mode == "gpx" && self.track_gpx_file.is_none() {
            anyhow::bail!("track_gpx_file is required for gpx track_mode");
        }
        Ok(())
    }

    /// Human-readable GPS connection string shown in TUI.
    pub fn connection_display(&self) -> String {
        format!("{} @ {} baud", self.port_name, self.baud_rate)
    }

    /// Human-readable MQTT broker address shown in TUI.
    pub fn mqtt_address(&self) -> String {
        format!("{}:{}", self.mqtt_host, self.mqtt_port)
    }
}

/// Load configuration from TOML file(s) and `GPS_TO_MQTT_*` environment variables.
///
/// Search order for config files (all found files are merged, later wins):
/// 1. `/usr/etc/g86-car-telemetry/gps-to-mqtt.toml`
/// 2. `/etc/g86-car-telemetry/gps-to-mqtt.toml`
/// 3. `/etc/gps-to-mqtt/settings.toml`
/// 4. `<exe-dir>/settings.toml` and `<exe-dir>/gps-to-mqtt.toml`
/// 5. `./settings.toml` and `./gps-to-mqtt.toml`
///
/// Environment variables (`GPS_TO_MQTT_PORT_NAME`, etc.) override everything.
pub fn load_configuration(config_path: Option<&str>) -> Result<AppConfig> {
    dotenvy::dotenv().ok();

    let mut builder = Config::builder();

    let loaded_from = if let Some(path) = config_path {
        builder = builder.add_source(File::with_name(path).required(true));
        Some(path.to_string())
    } else {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();

        for loc in &[
            "/usr/etc/g86-car-telemetry/gps-to-mqtt.toml",
            "/etc/g86-car-telemetry/gps-to-mqtt.toml",
            "/etc/gps-to-mqtt/settings.toml",
        ] {
            candidates.push(std::path::PathBuf::from(loc));
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join("settings.toml"));
                candidates.push(parent.join("gps-to-mqtt.toml"));
            }
        }

        candidates.push(std::path::PathBuf::from("./settings.toml"));
        candidates.push(std::path::PathBuf::from("./gps-to-mqtt.toml"));

        let mut found_path = None;
        for path in &candidates {
            if path.exists() {
                builder = builder.add_source(File::from(path.clone()).required(false));
                found_path = Some(path.display().to_string());
            }
        }

        if found_path.is_none() {
            warn!("No configuration file found; using defaults and environment variables");
        }

        found_path
    };

    // GPS_TO_MQTT_ environment variable overrides (highest priority)
    builder = builder.add_source(
        Environment::with_prefix("GPS_TO_MQTT")
            .separator("_")
            .try_parsing(true),
    );

    let settings = builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build configuration: {}", e))?;

    let mut app_config: AppConfig = settings
        .try_deserialize()
        .map_err(|e| anyhow::anyhow!("Failed to parse configuration: {}", e))?;

    app_config.config_path = loaded_from;
    app_config.validate()?;

    match &app_config.config_path {
        Some(path) => info!("Configuration loaded from: {}", path),
        None => info!("Configuration loaded from defaults and environment"),
    }

    Ok(app_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- defaults ---

    #[test]
    fn test_default_config_has_sensible_values() {
        let c = AppConfig::default();
        assert_eq!(c.baud_rate, 9600);
        assert_eq!(c.mqtt_host, "localhost");
        assert_eq!(c.mqtt_port, 1883);
        assert!(c.mqtt_enabled);
        assert_eq!(c.log_level, "info");
        assert_eq!(c.track_mode, "disabled");
        assert_eq!(c.tui_refresh_rate_ms, 100);
        assert!(!c.log_json);
        assert!(!c.set_gps_to_10hz);
    }

    // --- validation: port ---

    #[test]
    fn test_validate_default_config_passes() {
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_empty_port_name_fails() {
        let mut c = AppConfig::default();
        c.port_name = String::new();
        assert!(c.validate().is_err());
    }

    // --- validation: baud rate ---

    #[test]
    fn test_validate_invalid_baud_rate_fails() {
        let mut c = AppConfig::default();
        c.baud_rate = 12345;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_all_supported_baud_rates_pass() {
        for &rate in VALID_BAUD_RATES {
            let mut c = AppConfig::default();
            c.baud_rate = rate;
            assert!(c.validate().is_ok(), "baud_rate {} should be valid", rate);
        }
    }

    // --- validation: MQTT ---

    #[test]
    fn test_validate_mqtt_empty_host_when_enabled_fails() {
        let mut c = AppConfig::default();
        c.mqtt_enabled = true;
        c.mqtt_host = String::new();
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_mqtt_disabled_empty_host_passes() {
        let mut c = AppConfig::default();
        c.mqtt_enabled = false;
        c.mqtt_host = String::new();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_mqtt_port_zero_fails() {
        let mut c = AppConfig::default();
        c.mqtt_enabled = true;
        c.mqtt_port = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_mqtt_empty_base_topic_fails() {
        let mut c = AppConfig::default();
        c.mqtt_enabled = true;
        c.mqtt_base_topic = String::new();
        assert!(c.validate().is_err());
    }

    // --- validation: log level ---

    #[test]
    fn test_validate_invalid_log_level_fails() {
        let mut c = AppConfig::default();
        c.log_level = "verbose".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_all_valid_log_levels_pass() {
        for level in &["trace", "debug", "info", "warn", "error"] {
            let mut c = AppConfig::default();
            c.log_level = level.to_string();
            assert!(
                c.validate().is_ok(),
                "log_level '{}' should be valid",
                level
            );
        }
    }

    #[test]
    fn test_validate_log_level_case_insensitive() {
        let mut c = AppConfig::default();
        c.log_level = "INFO".to_string();
        assert!(c.validate().is_ok());
    }

    // --- validation: track mode ---

    #[test]
    fn test_validate_invalid_track_mode_fails() {
        let mut c = AppConfig::default();
        c.track_mode = "gps_auto".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_manual_mode_without_coords_fails() {
        let mut c = AppConfig::default();
        c.track_mode = "manual".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_manual_mode_with_coords_passes() {
        let mut c = AppConfig::default();
        c.track_mode = "manual".to_string();
        c.track_start_lat = Some(40.7128);
        c.track_start_lon = Some(-74.0060);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_gpx_mode_without_file_fails() {
        let mut c = AppConfig::default();
        c.track_mode = "gpx".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_gpx_mode_with_file_passes() {
        let mut c = AppConfig::default();
        c.track_mode = "gpx".to_string();
        c.track_gpx_file = Some("/path/to/track.gpx".to_string());
        assert!(c.validate().is_ok());
    }

    // --- helpers ---

    #[test]
    fn test_connection_display() {
        let mut c = AppConfig::default();
        c.port_name = "/dev/ttyUSB0".to_string();
        c.baud_rate = 115200;
        assert_eq!(c.connection_display(), "/dev/ttyUSB0 @ 115200 baud");
    }

    #[test]
    fn test_mqtt_address() {
        let mut c = AppConfig::default();
        c.mqtt_host = "broker.example.com".to_string();
        c.mqtt_port = 8883;
        assert_eq!(c.mqtt_address(), "broker.example.com:8883");
    }

    #[test]
    fn test_mqtt_address_default() {
        assert_eq!(AppConfig::default().mqtt_address(), "localhost:1883");
    }
}
