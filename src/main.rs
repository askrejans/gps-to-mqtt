mod config;
mod gps_data_parser;
mod mqtt_handler;
mod serial_port_handler;

use serial_port_handler::{read_from_port, setup_serial_port};
use gumdrop::Options;
use config::load_configuration;

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

/// Displays a graphical welcome message.
fn display_welcome() {
    println!("\nWelcome to GPS Data Processor!\n");
    println!("==========================================");
    println!("Press 'q' + Enter to quit the application.");
    println!("==========================================\n");
}

/// Define options for the program.
#[derive(Debug, Options)]
struct MyOptions {
    #[options(help = "print help message")]
    help: bool,

    #[options(help = "Sets a custom config file", meta = "FILE")]
    config: Option<String>,
}

fn print_help() {
    println!("Usage: gps-to-mqtt [options]");
    println!("Options:");
    println!("  -h, --help               Print this help message");
    println!("  -c, --config FILE        Sets a custom config file path");
}

fn main() {
    // Parse CLI arguments using gumdrop
    let opts = MyOptions::parse_args_default_or_exit();

    if opts.help {
        // Use custom print_help function to display help and exit
        print_help();
        std::process::exit(0);
    }

    // Display welcome message
    display_welcome();

    // Load configuration, set up serial port, and start processing
    let config_path = opts.config.as_deref();
    let config = match load_configuration(config_path) {
        Ok(config) => config,
        Err(err) => {
            // Handle the error gracefully, print a message, and exit
            eprintln!("Error loading configuration: {}", err);
            std::process::exit(1);
        }
    };

    let mut port = setup_serial_port(&config);
    read_from_port(&mut port, &config);
}