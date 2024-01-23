# GPS-to-MQTT

## Overview

This Rust project is designed to read data from a USB GPS dongle in NMEA-0183 format over serial, parse the data, and publish relevant information to an MQTT broker. Please note that this implementation is not complete, and it has only been tested with a specific USB dongle. There is no guarantee that it will work with different devices.

## Features

- Reads GPS data from a USB dongle in NMEA-0183 format.
- Parses relevant sentences and dispatches them to specialized functions.
- Publishes parsed information to MQTT topics.

## Warning

### Device Compatibility

This project has been tested with a specific USB dongle (TOPGNSS GN800G with M8030-KT chipset). Compatibility with other devices is not guaranteed.

### 10Hz Mode Toggle

There is a toggle that switches the dongle to 10Hz mode, which might be dangerous on other devices. Use this feature at your own risk. Binary commands with u-blox undocumented commands are pushed to the device for this operation.

## Main Logic

The main parsing logic is contained in the `gps_data_parser` module, specifically in the `process_gps_data` function. This function takes a slice of bytes representing received data, converts it to a string, and dispatches the relevant sentences to specialized parsing functions.

## Build Instructions

To build the project, follow these steps:

1. Ensure you have Rust installed on your system. If not, you can install it from [https://www.rust-lang.org/tools/install](https://www.rust-lang.org/tools/install).

2. Clone the repository:

    ```bash
    git clone https://github.com/your-username/gps-to-mqtt.git
    ```

3. Change into the project directory:

    ```bash
    cd gps-to-mqtt
    ```

4. Create an `example.settings.toml` file in the same directory as the executable. Refer to `example.settings.toml` for configuration options.

5. Build the project:

    ```bash
    cargo build --release
    ```

6. Run the executable:

    ```bash
    ./target/release/gps-to-mqtt
    ```

## Configuration

Copy and modify the `example.settings.toml` file to configure the project. Ensure that this file is in the same directory as the executable.

## Dependencies

- [serialport](https://crates.io/crates/serialport) - 4.3.0
- [config](https://crates.io/crates/config) - 0.13.4
- [paho-mqtt](https://crates.io/crates/paho-mqtt) - 0.12.3
- [futures](https://crates.io/crates/futures) - 0.3.30
- [lazy_static](https://crates.io/crates/lazy_static) - 1.4.0

## Project Structure

- `src/config.rs`: Module for loading project configuration.
- `src/gps_data_parser.rs`: Module containing the main logic for parsing GPS data.
- `src/mqtt_handler.rs`: Module for setting up MQTT and publishing messages.
- `src/serial_port_handler.rs`: Module for setting up and reading from the serial port.
- `src/main.rs`: Entry point for the application.

## Usage

1. Clone the repository and build the project using the provided build instructions.
2. Ensure that the USB GPS dongle is connected to the system.
3. Copy and modify the `example.settings.toml` file to configure the project.
4. Run the executable as described in the build instructions.

## MQTT data format

MQTT data is stored under configured topic as 3 letter codes:

- CRS - course in degrees
- TME - GMT time in HH:MM:SS format
- DTE - date in dd.mm.YYYY format
- LAT - latitude
- LNG - longitude
- SPD - speed in km/h
- ALT - altitude in m
- QTY - fix quality

![image](https://github.com/askrejans/gps-to-mqtt/assets/1042303/37bf6b97-259f-4e90-bbb2-71de8d6aeef1)


## License

This project is licensed under the [MIT License](LICENSE). Feel free to use, modify, and distribute the code as per the license terms.
