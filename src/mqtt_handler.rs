use log::{debug, error};
use paho_mqtt as mqtt;
use std::{process, time::Duration};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Invalid QoS level. Must be 0, 1 or 2")]
    InvalidQoS,
    #[error("MQTT error: {0}")]
    MqttError(#[from] mqtt::Error),
    #[error("Empty topic or payload")]
    EmptyInput,
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
    // Validate inputs
    if topic.is_empty() || payload.is_empty() {
        return Err(PublishError::EmptyInput);
    }

    if qos > 2 {
        return Err(PublishError::InvalidQoS);
    }

    debug!("Publishing message to topic: {}", topic);

    let msg = mqtt::MessageBuilder::new()
        .topic(topic)
        .payload(payload)
        .qos(qos)
        .retained(true)
        .finalize();

    cli.publish(msg).map_err(PublishError::MqttError)
}
