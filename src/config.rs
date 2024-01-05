use config::Config;

/// Struct to hold the application configuration.
pub struct AppConfig {
    /// The name of the serial port.
    pub port_name: String,

    /// The baud rate for the serial port.
    pub baud_rate: i64,

    //Should the GPS sample rate be increased to 10Hz
    pub set_gps_to_10hz: bool,

    /// The MQTT broker host address.
    pub mqtt_host: String,

    /// The MQTT broker port number.
    pub mqtt_port: i64,

    //The base topic of MQTT where data is pushed
    pub mqtt_base_topic: String,
}

/// Load application configuration from a TOML file.
///
/// This function reads the configuration settings from a TOML file named "settings.toml".
/// It expects the following keys in the TOML file: "port_name", "baud_rate", "mqtt_host", and "mqtt_port".
///
/// # Panics
/// Panics if any of the required configuration keys are missing or if there is an error reading the configuration file.
///
/// # Returns
/// Returns an `AppConfig` struct containing the loaded configuration.
pub fn load_configuration() -> AppConfig {
    // Build a new Config object with a file source.
    let settings = Config::builder()
        .add_source(config::File::with_name("settings.toml"))
        .build()
        .unwrap();

    // Create an AppConfig struct by extracting values from the configuration.
    AppConfig {
        port_name: settings
            .get_string("port_name")
            .expect("Missing port_name in configuration"),
        baud_rate: settings
            .get_int("baud_rate")
            .expect("Missing baud_rate in configuration"),
        set_gps_to_10hz: settings
            .get_bool("set_gps_to_10hz")
            .expect("Missing set_gps_to_10hz in configuration"),
        mqtt_host: settings
            .get_string("mqtt_host")
            .expect("Missing mqtt_host in configuration"),
        mqtt_port: settings
            .get_int("mqtt_port")
            .expect("Missing mqtt_port in configuration"),
        mqtt_base_topic: settings
            .get_string("mqtt_base_topic")
            .expect("Missing mqtt_base_topic in configuration"),
    }
}
