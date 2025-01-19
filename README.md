# GPS-to-MQTT

## Overview

This Rust project serves as a bridge between GPS hardware and MQTT-based systems, enabling real-time GPS data integration into IoT and telemetry applications. It reads NMEA-0183 format data from USB GPS dongles, processes the various sentence types, and publishes parsed information to configurable MQTT topics.

### Key Capabilities

- **GPS Data Processing**: Reads and parses standard NMEA-0183 sentences including position, speed, course, and satellite information
- **Real-time MQTT Publishing**: Converts GPS data into structured MQTT messages with configurable topics and QoS levels
- **High-Frequency Updates**: Optional support for 10Hz update rates on compatible u-blox GPS modules
- **Flexible Configuration**: TOML-based configuration for serial port settings, MQTT broker details, and topic customization

### Hardware Compatibility

While the software supports standard NMEA-0183 protocols, it has been primarily tested with the TOPGNSS GN800G GPS module (M8030-KT chipset). The 10Hz high-frequency mode specifically targets u-blox compatible devices. Users should exercise caution when using untested GPS hardware. Use it at your own risk!

### Use Cases

- Vehicle tracking systems
- Fleet management solutions
- IoT data collection
- Navigation applications
- Telemetry systems integration

> **Note**: This is an ongoing development project. While functional, it may require adjustments for specific use cases or hardware configurations. Contributions and feedback are welcome to improve compatibility and features.

## Features

- 📡 Reads NMEA-0183 GPS data from USB GPS dongles
- 🔄 Support for 10Hz GPS update rate (u-blox devices only)
- 🛰️ Parses multiple NMEA sentence types:
  - GSV (Satellites in View)
  - GGA (Fix Information)
  - RMC (Recommended Minimum Data)
  - VTG (Track & Speed)
  - GSA (Overall Satellite Data)
  - GLL (Geographic Position)
  - TXT (Text Transmission)
- 📊 Publishes parsed data to MQTT topics

### 10Hz Mode Toggle

There is a toggle that switches the dongle to 10Hz mode, which might be dangerous on other devices. Use this feature at your own risk. Binary commands with u-blox undocumented commands are pushed to the device for this operation.

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

4. Copy the [example.settings.toml] file to [settings.toml] in the same directory as the executable. Modify [settings.toml] as needed for your configuration:

    ```bash
    cp example.settings.toml settings.toml
    ```

5. Build the project in release mode:

    ```bash
    cargo build --release
    ```

6. Run the executable:

    ```bash
    ./target/release/gps-to-mqtt
    ```

## Project Structure

- `src/config.rs`: Module for loading project configuration.
- `src/gps_data_parser.rs`: Module containing the main logic for parsing GPS data.
- `src/mqtt_handler.rs`: Module for setting up MQTT and publishing messages.
- `src/serial_port_handler.rs`: Module for setting up and reading from the serial port.
- `src/main.rs`: Entry point for the application.

## MQTT Data Format

MQTT data is stored under the configured base topic (default: `/GOLF86/GPS/`) using 3-letter codes as subtopics.

### Core GPS Data
- `CRS` - Course/heading in degrees (0-359°)
- `TME` - GMT time in HH:MM:SS format
- `DTE` - Date in dd.mm.YYYY format
- `LAT` - Latitude in decimal degrees (±90°)
- `LNG` - Longitude in decimal degrees (±180°)
- `SPD` - Ground speed in km/h
- `ALT` - Altitude in meters above sea level
- `QTY` - GPS fix quality (0=invalid, 1=GPS fix, 2=DGPS fix)

### Additional Speed Formats
- `SPD_KTS` - Speed in knots
- `SPD_KPH` - Speed in kilometers per hour

### Satellite Information
- `SAT/GLOBAL/NUM` - Total number of satellites in view
- `SAT/GLOBAL/ANTSTATUS` - Antenna status
- `SAT/GLOBAL/PF` - Position fix status
- `SAT/GLOBAL/GNSS_OTP` - GNSS chip configuration

### Per-Satellite Data
Under `SAT/VEHICLES/{PRN}/` where PRN is the satellite ID:
- `FIX_TYPE` - Fix type (Not Available, 2D, 3D)
- Full satellite info string containing:
  - PRN number
  - Satellite type (GPS/GLONASS/Galileo/BeiDou)
  - Elevation angle
  - Azimuth angle
  - SNR (Signal-to-Noise Ratio)
  - In View status

### Geographic Position (GLL specific)
- `GLL_TME` - Time from GLL sentence
- `GLL_LAT` - Latitude from GLL sentence
- `GLL_LNG` - Longitude from GLL sentence

## Pre-Built Packages

There are also pre build packages (outdated), that combines three individual components: [Speeduino-to-MQTT](https://github.com/askrejans/speeduino-to-mqtt), [GPS-to-MQTT](https://github.com/askrejans/gps-to-mqtt), and [G86 Web Dashboard](https://github.com/askrejans/G86-web-dashboard) in one system with predefined services.

You can quickly get started by using pre-built packages available for both x64 and Raspberry Pi 4 (ARM) architectures:

- **DEB Packages for x64:** [Download here](https://akelaops.com/repo/deb/pool/main/amd64/g86-car-telemetry_1.0.deb)
- **DEB Packages for Raspberry Pi 4 (ARM):** [Download here](https://akelaops.com/repo/deb/pool/main/aarch64/g86-car-telemetry_1.0.deb)
- **RPM Packages for x64:** [Download here](https://akelaops.com/repo/rpm/x86_64/g86-car-telemetry-1.0-1.x86_64.rpm)
- **RPM Packages for Raspberry Pi 4 (ARM):** [Download here](https://akelaops.com/repo/rpm/aarch64/g86-car-telemetry-1.0-1.aarch64.rpm)

### Package Installation Details

- All packages install the three services in the directory `/opt/g86-car-telemetry` (or `/usr/opt/g86-car-telemetry`).
- Configuration files for GPS and ECU processors can be found under `/etc/g86-car-telemetry` (or `/usr/etc/g86-car-telemetry`).
- Web project configurations are located in `/var/www/g86-car-telemetry/config` (or `/usr/var/www/g86-car-telemetry/config`).
- Ensure to add relevant configurations for MQTT server, TTY ports, and any extra settings.

### Installed Services

The packages automatically install and manage the following services:

- `g86-car-telemetry-gps`
- `g86-car-telemetry-speeduino`
- `g86-car-telemetry-web`

### Compatibility and Testing

These packages have been tested on both Raspberry Pi 4 (ARM) with DEB packages and x86 systems with RPM packages. However, please note that this project is a work in progress, and more tests are needed, especially with real ECUs. Exercise caution when using, and stay tuned for updates as development continues to enhance and stabilize the functionality.

Feel free to reach out if you have any questions or encounter issues. Happy telemetry monitoring! 📊🛠️
## License

This project is licensed under the [MIT License](LICENSE). Feel free to use, modify, and distribute the code as per the license terms.
