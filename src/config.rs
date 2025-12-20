use crate::models::AppMode;
use anyhow::{Context, Result};
use config::{Config, File};
use std::path::Path;

/// Struct to hold the application configuration.
#[derive(Clone)]
pub struct AppConfig {
    // Application mode
    pub mode: AppMode,

    /// The name of the serial port.
    pub port_name: String,

    /// The baud rate for the serial port.
    pub baud_rate: u32,

    /// Should the GPS sample rate be increased to 10Hz
    pub set_gps_to_10hz: bool,

    /// The MQTT broker host address.
    pub mqtt_host: String,

    /// The MQTT broker port number.
    pub mqtt_port: u16,

    /// MQTT client ID
    pub mqtt_client_id: String,

    /// The base topic of MQTT where data is pushed
    pub mqtt_base_topic: String,

    /// Maximum MQTT reconnection attempts (0 = infinite)
    pub mqtt_reconnect_max_attempts: u32,

    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,

    /// Log file path (for service mode)
    pub log_file_path: Option<String>,

    /// TUI refresh rate in milliseconds
    pub tui_refresh_rate_ms: u64,

    /// Maximum log buffer size for TUI
    #[allow(dead_code)]
    pub max_log_buffer_size: usize,

    // Telemetry Configuration
    /// Enable telemetry calculations (acceleration, g-forces, etc.)
    pub telemetry_enabled: bool,

    /// Smoothing window size for telemetry calculations
    pub telemetry_smoothing_window: usize,

    /// Track configuration mode: "disabled", "manual", "learn", "gpx"
    pub track_mode: String,

    /// Start/Finish line latitude (for manual mode)
    pub track_start_lat: Option<f64>,

    /// Start/Finish line longitude (for manual mode)
    pub track_start_lon: Option<f64>,

    /// Start/Finish geofence radius in meters
    pub track_geofence_radius: f64,

    /// GPX file path (for gpx mode)
    pub track_gpx_file: Option<String>,
}

/// Load application configuration from a TOML file.
///
/// This function reads the configuration settings from a TOML file.
///
/// # Arguments
/// - `config_path`: An optional path to the configuration file.
/// - `mode`: The application mode (defaults to CLI if not specified in config)
///
/// # Returns
/// Returns a `Result` containing either the `AppConfig` struct with the loaded configuration or an error message.
pub fn load_configuration(config_path: Option<&str>, mode: AppMode) -> Result<AppConfig> {
    let settings = if let Some(path) = config_path {
        load_from_path(path)?
    } else {
        load_default_paths()?
    };

    Ok(AppConfig {
        mode,
        port_name: settings
            .get_string("port_name")
            .unwrap_or_else(|_| "/dev/ttyUSB0".to_string()),
        baud_rate: settings.get_int("baud_rate").unwrap_or(9600) as u32,
        set_gps_to_10hz: settings.get_bool("set_gps_to_10hz").unwrap_or(false),
        mqtt_host: settings
            .get_string("mqtt_host")
            .unwrap_or_else(|_| "localhost".to_string()),
        mqtt_port: settings.get_int("mqtt_port").unwrap_or(1883) as u16,
        mqtt_client_id: settings
            .get_string("mqtt_client_id")
            .unwrap_or_else(|_| "gps-to-mqtt".to_string()),
        mqtt_base_topic: settings
            .get_string("mqtt_base_topic")
            .unwrap_or_else(|_| "/gps".to_string()),
        mqtt_reconnect_max_attempts: settings
            .get_int("mqtt_reconnect_max_attempts")
            .unwrap_or(0) as u32,
        log_level: settings
            .get_string("log_level")
            .unwrap_or_else(|_| "info".to_string()),
        log_file_path: settings.get_string("log_file_path").ok(),
        tui_refresh_rate_ms: settings.get_int("tui_refresh_rate_ms").unwrap_or(100) as u64,
        max_log_buffer_size: settings
            .get_int("max_log_buffer_size")
            .unwrap_or(1000) as usize,
        telemetry_enabled: settings
            .get_bool("telemetry_enabled")
            .unwrap_or(true),
        telemetry_smoothing_window: settings
            .get_int("telemetry_smoothing_window")
            .unwrap_or(3) as usize,
        track_mode: settings
            .get_string("track_mode")
            .unwrap_or_else(|_| "disabled".to_string()),
        track_start_lat: settings.get_float("track_start_lat").ok(),
        track_start_lon: settings.get_float("track_start_lon").ok(),
        track_geofence_radius: settings
            .get_float("track_geofence_radius")
            .unwrap_or(15.0),
        track_gpx_file: settings.get_string("track_gpx_file").ok(),
    })
}

/// Loads the configuration from the specified path.
///
/// This function attempts to load the configuration from the given file path.
/// If the file is successfully loaded, the configuration is returned.
/// If there is an error loading the file, an error message is returned.
///
/// # Arguments
///
/// * `path` - A string slice that holds the path to the configuration file.
///
/// # Returns
///
/// * `Ok(Config)` - If the configuration file is successfully loaded.
/// * `Err(anyhow::Error)` - If there is an error loading the configuration file.
fn load_from_path(path: &str) -> Result<Config> {
    Config::builder()
        .add_source(File::with_name(path))
        .build()
        .context("Failed to load configuration file")
}

/// Attempts to load the configuration from default paths.
///
/// This function tries to load the configuration from the following locations in order:
/// 1. A `settings.toml` file located in the same directory as the executable.
/// 2. A `gps-to-mqtt.toml` file located at `/usr/etc/g86-car-telemetry/`.
/// 3. A `gps-to-mqtt.toml` file located at `/etc/g86-car-telemetry/`.
///
/// If a configuration file is successfully loaded from any of these locations, it will be used.
/// If none of the files are found or successfully loaded, the default configuration will be returned.
///
/// # Returns
///
/// * `Ok(Config)` - If a configuration file is successfully loaded from any of the default paths.
/// * `Err(anyhow::Error)` - If there is an error loading the configuration from all default paths.
fn load_default_paths() -> Result<Config> {
    if let Ok(exe_dir) = std::env::current_exe() {
        let exe_dir = exe_dir.parent().unwrap_or_else(|| Path::new("."));
        let default_path = exe_dir.join("settings.toml");

        if let Ok(config) = Config::builder()
            .add_source(File::with_name(default_path.to_str().unwrap()))
            .build()
        {
            return Ok(config);
        }
    }

    if let Ok(config) = Config::builder()
        .add_source(File::with_name(
            "/usr/etc/g86-car-telemetry/gps-to-mqtt.toml",
        ))
        .build()
    {
        return Ok(config);
    }

    if let Ok(config) = Config::builder()
        .add_source(File::with_name("/etc/g86-car-telemetry/gps-to-mqtt.toml"))
        .build()
    {
        return Ok(config);
    }

    Ok(Config::default())
}
