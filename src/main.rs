/// # GPS Data Processor
///
/// This Rust application serves as a processor for GPS data, converting it to MQTT messages.
/// It includes modules for configuration, GPS data parsing, serial port handling, and MQTT handling.
/// The main function loads configuration, sets up serial communication, and starts reading data from the port.
///
/// ## Usage
///
/// Simply run the application, and it will establish communication with the GPS device. Press 'q' to quit the application.
///
/// ## Modules
///
/// - `config`: Module for configuration settings.
/// - `gps_data_parser`: Module for parsing GPS data.
/// - `mqtt_handler`: Module for handling MQTT communication.
/// - `serial_port_handler`: Module for handling serial communication with the GPS device.
///
/// ## Functions
///
/// - `main()`: The main function that loads configuration, sets up serial communication, and starts reading data from the port.
/// - `display_welcome()`: Function to display a graphical welcome message.
mod config;
mod gps_data_parser;
mod mqtt_handler;
mod serial_port_handler;

use config::load_configuration;
use serial_port_handler::{read_from_port, setup_serial_port};

/// Displays a graphical welcome message.
fn display_welcome() {
    println!("\nWelcome to GPS Data Processor!\n");
    println!("===================================");
}

fn main() {
    // Display welcome message
    display_welcome();

    // Load configuration, set up serial port, and start processing
    let config = load_configuration();
    let mut port = setup_serial_port(&config);
    read_from_port(&mut port, &config);
}
