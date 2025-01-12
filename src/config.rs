use config::{Config, File};
use std::path::Path;

/// Struct to hold the application configuration.
pub struct AppConfig {
    /// The name of the serial port.
    pub port_name: String,

    /// The baud rate for the serial port.
    pub baud_rate: i64,

    // Should the GPS sample rate be increased to 10Hz
    pub set_gps_to_10hz: bool,

    /// The MQTT broker host address.
    pub mqtt_host: String,

    /// The MQTT broker port number.
    pub mqtt_port: i64,

    // The base topic of MQTT where data is pushed
    pub mqtt_base_topic: String,

    // Optional: Path to the configuration file
    pub config_path: Option<String>,
}

/// Load application configuration from a TOML file.
///
/// This function reads the configuration settings from a TOML file.
///
/// # Arguments
/// - `config_path`: An optional path to the configuration file.
///
/// # Returns
/// Returns a `Result` containing either the `AppConfig` struct with the loaded configuration or an error message.
pub fn load_configuration(config_path: Option<&str>) -> Result<AppConfig, String> {
    let mut settings = Config::default();

    if let Some(path) = config_path {
        settings = load_from_path(path)?;
    } else {
        settings = load_default_paths()?;
    }

    Ok(AppConfig {
        port_name: settings
            .get_string("port_name")
            .unwrap_or_else(|_| "default_port".to_string()),
        baud_rate: settings.get_int("baud_rate").unwrap_or(9600),
        set_gps_to_10hz: settings.get_bool("set_gps_to_10hz").unwrap_or(false),
        mqtt_host: settings
            .get_string("mqtt_host")
            .unwrap_or_else(|_| "default_host".to_string()),
        mqtt_port: settings.get_int("mqtt_port").unwrap_or(1883),
        mqtt_base_topic: settings
            .get_string("mqtt_base_topic")
            .unwrap_or_else(|_| "default_topic".to_string()),
        config_path: config_path.map(|p| p.to_string()),
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
/// * `Err(String)` - If there is an error loading the configuration file.
fn load_from_path(path: &str) -> Result<Config, String> {
    Config::builder()
        .add_source(File::with_name(path))
        .build()
        .map_err(|err| format!("{}", err))
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
/// * `Err(String)` - If there is an error loading the configuration from all default paths.
fn load_default_paths() -> Result<Config, String> {
    let mut settings = Config::default();

    if let Ok(exe_dir) = std::env::current_exe() {
        let exe_dir = exe_dir.parent().unwrap_or_else(|| Path::new("."));
        let default_path = exe_dir.join("settings.toml");

        if let Ok(config) = Config::builder()
            .add_source(File::with_name(default_path.to_str().unwrap()))
            .build()
        {
            settings = config;
        }
    }

    if let Err(_) = Config::builder()
        .add_source(File::with_name(
            "/usr/etc/g86-car-telemetry/gps-to-mqtt.toml",
        ))
        .build()
        .and_then(|config| {
            settings = config;
            Ok(())
        })
    {
        if let Ok(config) = Config::builder()
            .add_source(File::with_name("/etc/g86-car-telemetry/gps-to-mqtt.toml"))
            .build()
        {
            settings = config;
        }
    }

    Ok(settings)
}
