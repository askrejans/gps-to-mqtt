use log::{debug, error};
use paho_mqtt as mqtt;
use std::collections::HashMap;
use std::sync::Mutex;
use std::{process, time::Duration};
use thiserror::Error;

lazy_static::lazy_static! {
    static ref LAST_VALUES: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Invalid QoS level. Must be 0, 1 or 2")]
    InvalidQoS,
    #[error("MQTT error: {0}")]
    MqttError(#[from] mqtt::Error),
    #[error("Empty topic or payload")]
    EmptyInput,
    #[error("Mutex lock error")]
    LockError,
}

use crate::config::AppConfig;
use crate::models::{LapData, TelemetryMetrics};

/// Set up and return an MQTT client based on the provided configuration.
///
/// This function takes an `AppConfig` reference, extracts MQTT-related information
/// (host and port) from it, creates an MQTT client, sets a timeout, and attempts to connect to the broker.
///
/// # Arguments
///
/// * `config` - A reference to the `AppConfig` struct containing MQTT configuration information.
///
/// # Panics
///
/// Panics if there is an error creating the MQTT client or if it fails to connect to the broker.
///
/// # Returns
///
/// Returns an MQTT client upon successful setup and connection.
pub fn setup_mqtt(config: &AppConfig) -> mqtt::Client {
    // Format the MQTT broker host and port.
    let host = format!("mqtt://{}:{}", config.mqtt_host, config.mqtt_port);

    // Create an MQTT client.
    let mut cli = mqtt::Client::new(host).unwrap_or_else(|e| {
        // Print an error message and exit the program if client creation fails.
        println!("Error creating the client: {:?}", e);
        process::exit(1);
    });

    // Set a timeout of 5 seconds for synchronous calls.
    cli.set_timeout(Duration::from_secs(5));

    // Attempt to connect to the MQTT broker and exit the program if the connection fails.
    if let Err(e) = cli.connect(None) {
        println!("Unable to connect: {:?}", e);
        process::exit(1);
    }

    // Return the configured and connected MQTT client.
    cli
}

/// Publish an MQTT message only if the value has changed since last publication
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client
/// * `topic` - The MQTT topic to publish to
/// * `payload` - The message payload
/// * `qos` - Quality of Service level (0, 1, or 2)
///
/// # Returns
///
/// Returns `Result<(), PublishError>` indicating success or if an error occurred
pub fn publish_if_changed(
    cli: &mqtt::Client,
    topic: &str,
    payload: &str,
    qos: i32,
) -> Result<(), PublishError> {
    // Validate inputs
    if topic.is_empty() || payload.is_empty() {
        return Err(PublishError::EmptyInput);
    }

    if qos > 2 {
        return Err(PublishError::InvalidQoS);
    }

    // Get lock on last values
    let mut last_values = LAST_VALUES.lock().map_err(|_| PublishError::LockError)?;

    // Check if value has changed
    if last_values
        .get(topic)
        .map_or(true, |last_value| last_value != payload)
    {
        debug!("Publishing changed value to topic: {}", topic);

        // Create and publish message
        let msg = mqtt::MessageBuilder::new()
            .topic(topic)
            .payload(payload)
            .qos(qos)
            .retained(true)
            .finalize();

        cli.publish(msg).map_err(PublishError::MqttError)?;

        // Update stored value after successful publish
        last_values.insert(topic.to_string(), payload.to_string());

        Ok(())
    } else {
        debug!("Skipping publish - value unchanged for topic: {}", topic);
        Ok(())
    }
}

/// Publish an MQTT message to the specified topic with the given payload and QoS.
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client.
/// * `topic` - The MQTT topic to which the message will be published.
/// * `payload` - The payload of the MQTT message.
/// * `qos` - The Quality of Service level for the message.
///
/// # Returns
///
/// Returns `Result<(), mqtt::Error>` indicating success or failure.
pub fn publish_message(
    cli: &mqtt::Client,
    topic: &str,
    payload: &str,
    qos: i32,
) -> Result<(), PublishError> {
    // For backwards compatibility, this now calls publish_if_changed
    publish_if_changed(cli, topic, payload, qos)
}

/// Publish telemetry metrics to MQTT
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client
/// * `base_topic` - The base MQTT topic (e.g., "/gps")
/// * `metrics` - The telemetry metrics to publish
/// * `qos` - Quality of Service level
///
/// # Returns
///
/// Returns `Result<(), PublishError>` indicating success or failure
pub fn publish_telemetry(
    cli: &mqtt::Client,
    base_topic: &str,
    metrics: &TelemetryMetrics,
    qos: i32,
) -> Result<(), PublishError> {
    // Publish longitudinal acceleration
    if let Some(accel) = metrics.longitudinal_accel {
        let topic = format!("{}/ACCELERATION", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.3}", accel), qos)?;
    }

    // Publish lateral acceleration (g-forces)
    if let Some(lateral) = metrics.lateral_accel {
        let topic = format!("{}/LATERAL_G", base_topic);
        let lateral_g = lateral / 9.81; // Convert m/s² to g
        publish_if_changed(cli, &topic, &format!("{:.3}", lateral_g), qos)?;
    }

    // Publish combined g-force
    if let Some(combined) = metrics.combined_g {
        let topic = format!("{}/COMBINED_G", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.3}", combined), qos)?;
    }

    // Publish heading rate
    if let Some(heading_rate) = metrics.heading_rate {
        let topic = format!("{}/HEADING_RATE", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.2}", heading_rate), qos)?;
    }

    // Publish distance traveled
    let topic = format!("{}/DISTANCE", base_topic);
    publish_if_changed(cli, &topic, &format!("{:.1}", metrics.distance_traveled), qos)?;

    // Publish max speed
    if let Some(max_speed) = metrics.max_speed_kph {
        let topic = format!("{}/MAX_SPEED", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.1}", max_speed), qos)?;
    }

    // Publish braking status
    let topic = format!("{}/BRAKING", base_topic);
    publish_if_changed(cli, &topic, if metrics.is_braking { "1" } else { "0" }, qos)?;

    Ok(())
}

/// Publish lap timing data to MQTT
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client
/// * `base_topic` - The base MQTT topic (e.g., "/gps")
/// * `lap_data` - The lap data to publish
/// * `qos` - Quality of Service level
///
/// # Returns
///
/// Returns `Result<(), PublishError>` indicating success or failure
pub fn publish_lap_data(
    cli: &mqtt::Client,
    base_topic: &str,
    lap_data: &LapData,
    qos: i32,
) -> Result<(), PublishError> {
    // Publish lap number
    let topic = format!("{}/LAP_NUMBER", base_topic);
    publish_if_changed(cli, &topic, &lap_data.lap_number.to_string(), qos)?;

    // Publish current lap time
    if let Some(lap_time) = lap_data.lap_time_ms {
        let topic = format!("{}/LAP_TIME", base_topic);
        let lap_time_sec = lap_time as f64 / 1000.0;
        publish_if_changed(cli, &topic, &format!("{:.3}", lap_time_sec), qos)?;
    }

    // Publish best lap time
    if let Some(best_lap) = lap_data.best_lap_ms {
        let topic = format!("{}/BEST_LAP", base_topic);
        let best_lap_sec = best_lap as f64 / 1000.0;
        publish_if_changed(cli, &topic, &format!("{:.3}", best_lap_sec), qos)?;
    }

    // Publish sector times
    for (i, sector_time) in lap_data.sector_times_ms.iter().enumerate() {
        if let Some(time) = sector_time {
            let topic = format!("{}/SECTOR_{}", base_topic, i + 1);
            let sector_sec = *time as f64 / 1000.0;
            publish_if_changed(cli, &topic, &format!("{:.3}", sector_sec), qos)?;
        }
    }

    Ok(())
}

/// Publish DOP values (from FixData) to MQTT
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client
/// * `base_topic` - The base MQTT topic (e.g., "/gps")
/// * `vdop` - Vertical Dilution of Precision
/// * `pdop` - Position Dilution of Precision
/// * `qos` - Quality of Service level
///
/// # Returns
///
/// Returns `Result<(), PublishError>` indicating success or failure
pub fn publish_dop_values(
    cli: &mqtt::Client,
    base_topic: &str,
    vdop: Option<f64>,
    pdop: Option<f64>,
    qos: i32,
) -> Result<(), PublishError> {
    if let Some(vdop_val) = vdop {
        let topic = format!("{}/VDOP", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.2}", vdop_val), qos)?;
    }

    if let Some(pdop_val) = pdop {
        let topic = format!("{}/PDOP", base_topic);
        publish_if_changed(cli, &topic, &format!("{:.2}", pdop_val), qos)?;
    }

    Ok(())
}

/// Publish position accuracy from GST sentence
///
/// # Arguments
///
/// * `cli` - A reference to the MQTT client
/// * `base_topic` - The base MQTT topic (e.g., "/gps")
/// * `std_lat` - Standard deviation of latitude error (meters)
/// * `std_lon` - Standard deviation of longitude error (meters)
/// * `std_alt` - Standard deviation of altitude error (meters)
/// * `qos` - Quality of Service level
///
/// # Returns
///
/// Returns `Result<(), PublishError>` indicating success or failure
pub fn publish_position_accuracy(
    cli: &mqtt::Client,
    base_topic: &str,
    std_lat: f64,
    std_lon: f64,
    std_alt: f64,
    qos: i32,
) -> Result<(), PublishError> {
    // Calculate overall position accuracy (2D RMS)
    let position_accuracy = (std_lat * std_lat + std_lon * std_lon).sqrt();
    
    let topic = format!("{}/POSITION_ACCURACY", base_topic);
    publish_if_changed(cli, &topic, &format!("{:.2}", position_accuracy), qos)?;

    // Also publish individual components for detailed analysis
    let topic_lat = format!("{}/ACCURACY_LAT", base_topic);
    publish_if_changed(cli, &topic_lat, &format!("{:.2}", std_lat), qos)?;

    let topic_lon = format!("{}/ACCURACY_LON", base_topic);
    publish_if_changed(cli, &topic_lon, &format!("{:.2}", std_lon), qos)?;

    let topic_alt = format!("{}/ACCURACY_ALT", base_topic);
    publish_if_changed(cli, &topic_alt, &format!("{:.2}", std_alt), qos)?;

    Ok(())
}
