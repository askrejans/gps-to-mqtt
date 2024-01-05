mod config;
mod gps_data_parser;
mod mqtt_handler;
mod serial_port_handler;

use config::load_configuration;
use serial_port_handler::{read_from_port, setup_serial_port};

fn main() {
    let config = load_configuration();
    let mut port = setup_serial_port(&config);
    read_from_port(&mut port, &config);
}
