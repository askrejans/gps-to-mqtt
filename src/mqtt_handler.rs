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
