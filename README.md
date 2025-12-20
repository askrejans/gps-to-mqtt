# GPS-to-MQTT

## Overview

This Rust project serves as a bridge between GPS hardware and MQTT-based systems, enabling real-time GPS data integration into IoT and telemetry applications. It reads NMEA-0183 format data from USB GPS dongles, processes the various sentence types, and publishes parsed information to configurable MQTT topics.

### Key Capabilities

- **GPS Data Processing**: Reads and parses standard NMEA-0183 sentences including position, speed, course, and satellite information
- **Real-time MQTT Publishing**: Converts GPS data into structured MQTT messages with configurable topics and QoS levels
- **High-Frequency Updates**: Optional support for 10Hz update rates on compatible u-blox GPS modules
- **Flexible Configuration**: TOML-based configuration for serial port settings, MQTT broker details, and topic customization
- **Multiple Operational Modes**: TUI (interactive), CLI (minimal), and Service (daemon) modes
- **Racing Telemetry**: Advanced telemetry calculations including acceleration, g-forces, lap timing, and track analysis

### Hardware Compatibility

While the software supports standard NMEA-0183 protocols, it has been primarily tested with the TOPGNSS GN800G GPS module (M8030-KT chipset). The 10Hz high-frequency mode specifically targets u-blox compatible devices. Users should exercise caution when using untested GPS hardware. Use it at your own risk!

### Use Cases

- Vehicle tracking systems
- Fleet management solutions
- Car racing telemetry and lap timing
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
  - GNS (GNSS Fix Data)
  - GST (Position Accuracy)
  - ROT (Rate of Turn)
  - HDT (True Heading)
  - PUBX (u-blox Proprietary)
- 🏁 Racing telemetry features:
  - Real-time acceleration and g-force calculations
  - Lap timing with sector support
  - Braking detection and analysis
  - Distance tracking
- 📊 Publishes parsed data to MQTT topics
- 🖥️ Interactive TUI with satellite visualization
- 🌍 Multi-GNSS support (GPS, GLONASS, Galileo, BeiDou)

### 10Hz Mode Toggle

There is a toggle that switches the dongle to 10Hz mode, which might be dangerous on other devices. Use this feature at your own risk. Binary commands with u-blox undocumented commands are pushed to the device for this operation.

## Build Instructions

To build the project, follow these steps:

1. Ensure you have Rust installed on your system. If not, you can install it from [https://www.rust-lang.org/tools/install](https://www.rust-lang.org/tools/install).

2. Clone the repository:

    ```bash
    git clone https://github.com/askrejans/gps-to-mqtt.git
    ```

3. Change into the project directory:

    ```bash
    cd gps-to-mqtt
    ```

4. Copy the `example.settings.toml` file to `settings.toml` in the same directory as the executable. Modify `settings.toml` as needed for your configuration:

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

## Installation

### From Source

```bash
cargo build --release
sudo cp target/release/gps-to-mqtt /opt/gps-to-mqtt/
sudo cp example.settings.toml /etc/g86-car-telemetry/gps-to-mqtt.toml
```

### As a Systemd Service

```bash
# Copy the service file
sudo cp gps-to-mqtt.service /etc/systemd/system/

# Create user and directories
sudo useradd -r -s /bin/false gps
sudo mkdir -p /opt/gps-to-mqtt
sudo mkdir -p /var/log/gps-to-mqtt
sudo chown gps:gps /var/log/gps-to-mqtt

# Enable and start the service
sudo systemctl daemon-reload
sudo systemctl enable gps-to-mqtt
sudo systemctl start gps-to-mqtt
```

## Usage

### TUI Mode (Default for interactive use)

```bash
gps-to-mqtt --mode tui
```

**TUI Controls:**
- `1` or `Left/Right`: Switch between tabs
- `q` or `ESC`: Quit application

**TUI Tabs:**
- **Overview**: GPS position, speed, altitude, fix information, and messages
- **Satellites**: Detailed satellite list and sky view chart
- **Logs**: Real-time application logs with color coding

### CLI Mode

```bash
gps-to-mqtt --mode cli --config /path/to/config.toml
```

Use this mode for debugging or when you want minimal terminal output with structured logging.

### Service Mode

```bash
gps-to-mqtt --mode service --config /etc/g86-car-telemetry/gps-to-mqtt.toml
```

This mode is designed for running as a systemd service with JSON logging to files or journald.

## Project Structure

- `src/config.rs`: Module for loading project configuration
- `src/parser.rs`: NMEA sentence parser with support for multiple sentence types
- `src/telemetry.rs`: Racing telemetry calculations (acceleration, g-forces, distance)
- `src/track.rs`: Lap timing and track management
- `src/mqtt.rs`: Async MQTT client with reconnection logic
- `src/serial.rs`: Serial port handling with automatic reconnection
- `src/models.rs`: Data structures for GPS and telemetry state
- `src/ui/`: Terminal UI implementation with multiple tabs
- `src/service.rs`: Service mode and signal handling
- `src/logging.rs`: Mode-specific logging configuration
- `src/main.rs`: Entry point for the application

## Configuration

Configuration is loaded from TOML files. The application searches for configuration in these locations:

1. Path specified by `--config` argument
2. `./settings.toml` (next to executable)
3. `/usr/etc/g86-car-telemetry/gps-to-mqtt.toml`
4. `/etc/g86-car-telemetry/gps-to-mqtt.toml`

### Configuration Options

```toml
# Serial Port Configuration
port_name = "/dev/ttyACM0"              # Serial port device path
baud_rate = 9600                        # Serial baud rate
set_gps_to_10hz = false                 # Enable 10Hz mode (u-blox specific)

# MQTT Configuration
mqtt_host = "localhost"                 # MQTT broker hostname
mqtt_port = 1883                        # MQTT broker port
mqtt_client_id = "gps-to-mqtt"          # MQTT client identifier
mqtt_base_topic = "/GOLF86/GPS"         # Base topic for all GPS data
mqtt_reconnect_max_attempts = 0         # Max reconnection attempts (0 = infinite)

# Logging Configuration
log_level = "info"                      # Log level: trace, debug, info, warn, error
log_file_path = "/var/log/gps-to-mqtt/gps-to-mqtt.log"  # Log file path (service mode)

# TUI Configuration
tui_refresh_rate_ms = 100               # TUI refresh rate in milliseconds
max_log_buffer_size = 1000              # Maximum log entries in TUI

# Telemetry Configuration
telemetry_enabled = true                # Enable telemetry calculations
telemetry_smoothing_window = 3          # Number of samples for moving average

# Track/Lap Timing Configuration
track_mode = "disabled"                 # Options: "disabled", "manual", "learn", "gpx"
track_geofence_radius = 15.0            # Geofence radius in meters
# track_start_lat = 40.7128             # Start/Finish latitude (for manual mode)
# track_start_lon = -74.0060            # Start/Finish longitude (for manual mode)
# track_gpx_file = "/path/to/track.gpx" # GPX file path (for gpx mode)
```

## MQTT Topics

Data is published to the following topics (with configurable base):

### Core GPS Data
- `{base_topic}/LAT` - Latitude (decimal degrees)
- `{base_topic}/LON` - Longitude (decimal degrees)
- `{base_topic}/ALT` - Altitude (meters)
- `{base_topic}/SPEED` - Speed (km/h)
- `{base_topic}/COURSE` - Course/heading (degrees)
- `{base_topic}/SATS` - Satellites used in fix
- `{base_topic}/SATS_IN_VIEW` - Total satellites in view
- `{base_topic}/HDOP` - Horizontal dilution of precision
- `{base_topic}/VDOP` - Vertical dilution of precision
- `{base_topic}/PDOP` - Position dilution of precision
- `{base_topic}/TIME` - GPS time (HH:MM:SS)
- `{base_topic}/DATE` - GPS date (YYYY-MM-DD)

### Telemetry Data (Racing Features)
- `{base_topic}/ACCELERATION` - Longitudinal acceleration (m/s²)
- `{base_topic}/LATERAL_G` - Lateral g-force (cornering)
- `{base_topic}/COMBINED_G` - Total g-force magnitude
- `{base_topic}/HEADING_RATE` - Rate of heading change (deg/s)
- `{base_topic}/DISTANCE` - Total distance traveled (meters)
- `{base_topic}/MAX_SPEED` - Maximum speed recorded (km/h)
- `{base_topic}/BRAKING` - Braking status (0/1)

### Lap Timing Data
- `{base_topic}/LAP_NUMBER` - Current lap number
- `{base_topic}/LAP_TIME` - Last lap time (seconds)
- `{base_topic}/BEST_LAP` - Best lap time (seconds)
- `{base_topic}/SECTOR_1`, `/SECTOR_2`, etc. - Sector times (seconds)

### Position Accuracy
- `{base_topic}/POSITION_ACCURACY` - Overall position accuracy (meters)
- `{base_topic}/ACCURACY_LAT` - Latitude accuracy std deviation
- `{base_topic}/ACCURACY_LON` - Longitude accuracy std deviation
- `{base_topic}/ACCURACY_ALT` - Altitude accuracy std deviation
- `{base_topic}/TRUE_HEADING` - True heading (degrees)

All messages are published with QoS 0 and retained flag for last known values.

## Pre-Built Packages

There are also pre-built packages (may be outdated), that combine three individual components: [Speeduino-to-MQTT](https://github.com/askrejans/speeduino-to-mqtt), [GPS-to-MQTT](https://github.com/askrejans/gps-to-mqtt), and [G86 Web Dashboard](https://github.com/askrejans/G86-web-dashboard) in one system with predefined services.

You can quickly get started by using pre-built packages available for both x64 and Raspberry Pi 4 (ARM) architectures:

- **DEB Packages for x64:** [Download here](https://akelaops.com/repo/deb/pool/main/amd64/g86-car-telemetry_1.0.deb)
- **DEB Packages for Raspberry Pi 4 (ARM):** [Download here](https://akelaops.com/repo/deb/pool/main/aarch64/g86-car-telemetry_1.0.deb)
- **RPM Packages for x64:** [Download here](https://akelaops.com/repo/rpm/x86_64/g86-car-telemetry-1.0-1.x86_64.rpm)
- **RPM Packages for Raspberry Pi 4 (ARM):** [Download here](https://akelaops.com/repo/rpm/aarch64/g86-car-telemetry-1.0-1.aarch64.rpm)

### Package Installation Details

- All packages install the three services in the directory `/opt/g86-car-telemetry` (or `/usr/opt/g86-car-telemetry`)
- Configuration files for GPS and ECU processors can be found under `/etc/g86-car-telemetry` (or `/usr/etc/g86-car-telemetry`)
- Web project configurations are located in `/var/www/g86-car-telemetry/config` (or `/usr/var/www/g86-car-telemetry/config`)
- Ensure to add relevant configurations for MQTT server, TTY ports, and any extra settings

### Installed Services

The packages automatically install and manage the following services:

- `g86-car-telemetry-gps`
- `g86-car-telemetry-speeduino`
- `g86-car-telemetry-web`

### Compatibility and Testing

These packages have been tested on both Raspberry Pi 4 (ARM) with DEB packages and x86 systems with RPM packages. However, please note that this project is a work in progress, and more tests are needed, especially with real ECUs. Exercise caution when using, and stay tuned for updates as development continues to enhance and stabilize the functionality.

Feel free to reach out if you have any questions or encounter issues. Happy telemetry monitoring! 📊🛠️

## Architecture

The application uses a modern async architecture built on tokio:

```
Serial Port Task (blocking) → GPS Parser → Event Channel
                                              ↓
                                     State Aggregator
                                              ↓
                                        Shared State ← TUI/CLI/Service
                                              ↓
                                       MQTT Publisher
```

### Key Components

- **parser.rs**: Pure NMEA sentence parser returning structured events
- **serial.rs**: Async serial port handler with reconnection logic
- **mqtt.rs**: Async MQTT client with automatic reconnection and buffering
- **models.rs**: Data structures for GPS state and application state
- **ui/**: Ratatui-based TUI with multiple widgets and tabs
- **service.rs**: Signal handling for graceful shutdown
- **logging.rs**: Mode-specific logging configuration

## Development

### Build

```bash
cargo build
```

### Run Tests

```bash
cargo test
```

### Run with Logging

```bash
RUST_LOG=debug cargo run -- --mode tui
```

### Cross-Compilation for ARM (Raspberry Pi)

```bash
# Install cross-compilation toolchain
rustup target add armv7-unknown-linux-gnueabihf

# Build
cargo build --release --target armv7-unknown-linux-gnueabihf
```

## License

This project is licensed under the [MIT License](LICENSE). Feel free to use, modify, and distribute the code as per the license terms.

## Contributing

Contributions are welcome! Please submit issues and pull requests on GitHub.
