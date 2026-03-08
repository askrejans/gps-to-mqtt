# GPS-to-MQTT

## Overview

This Rust project serves as a bridge between GPS hardware and MQTT-based systems, enabling real-time GPS data integration into IoT and telemetry applications. It reads NMEA-0183 format data from USB GPS dongles, processes the various sentence types, and publishes parsed information to configurable MQTT topics.

![gps](https://github.com/user-attachments/assets/14f42bc3-59a6-4973-99da-32b818c8b44e)


### Key Capabilities

- **GPS Data Processing**: Reads and parses standard NMEA-0183 sentences including position, speed, course, and satellite information
- **Dual Input Modes**: Connect via a local serial port **or** a TCP bridge (e.g. [io-to-net](https://github.com/askrejans/io-to-net)) — configurable with a single setting
- **Real-time MQTT Publishing**: Converts GPS data into structured MQTT messages with configurable topics and QoS levels
- **High-Frequency Updates**: Optional support for 10Hz update rates on compatible u-blox GPS modules
- **Flexible Configuration**: TOML-based configuration for serial/TCP settings, MQTT broker details, and topic customization
- **Automatic Mode Detection**: Runs the interactive TUI when attached to a terminal; falls back to structured service logging when run as a daemon
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

- 📡 Reads NMEA-0183 GPS data from USB GPS dongles or a TCP bridge
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
sudo install -Dm755 target/release/gps-to-mqtt /usr/bin/gps-to-mqtt
sudo install -Dm644 example.settings.toml /etc/gps-to-mqtt/settings.toml
```

### As a Systemd Service

```bash
sudo cp gps-to-mqtt.service /lib/systemd/system/
sudo useradd --system --no-create-home --shell /usr/sbin/nologin gps
sudo systemctl daemon-reload
sudo systemctl enable --now gps-to-mqtt
```

### Build DEB / RPM Packages

Requires [`cross`](https://github.com/cross-rs/cross) and [`fpm`](https://fpm.readthedocs.io/):

```bash
cargo install cross
gem install fpm

# Build all packages for both architectures
./scripts/build_packages.sh

# Or target a specific arch/format:
./scripts/build_packages.sh --arch arm64 --type deb
./scripts/build_packages.sh --arch x86-64 --type rpm
```

Packages are written to `dist/`.

## Usage

### Interactive (TUI) mode

When the process is attached to a terminal the interactive four-tab dashboard starts automatically:

```bash
gps-to-mqtt
gps-to-mqtt --config /path/to/settings.toml
```

**TUI Controls:**
| Key | Action |
|-----|--------|
| `1` / `2` / `3` / `4` | Switch tabs directly |
| `Left` / `Right` | Cycle tabs |
| `q` / `Ctrl-C` | Quit |

**TUI Tabs:**
- **Overview (1)**: Connection status panel + live GPS data (position, fix, heading)
- **Satellites (2)**: Satellite list and sky-view chart
- **App Logs (3)**: Scrolling log ring-buffer
- **Raw GPS (4)**: Colour-coded NMEA sentences

### Service / daemon mode

When run with stdout redirected (e.g. as a systemd unit) the TUI is skipped and structured logs are written to stdout/journald:

```bash
gps-to-mqtt --config /etc/gps-to-mqtt/settings.toml
```

See `gps-to-mqtt.service` for a ready-made unit file.

## Project Structure

- `src/config.rs`: Module for loading project configuration
- `src/parser.rs`: NMEA sentence parser with support for multiple sentence types
- `src/telemetry.rs`: Racing telemetry calculations (acceleration, g-forces, distance)
- `src/track.rs`: Lap timing and track management
- `src/mqtt.rs`: Async MQTT client with reconnection logic
- `src/serial.rs`: Serial port handling with automatic reconnection
- `src/tcp.rs`: TCP bridge connection with automatic reconnection
- `src/metrics.rs`: Prometheus metrics exposition and health endpoint
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

See `example.settings.toml` for the full annotated reference. Common options:

```toml
# GPS input source: "serial" (default) or "tcp"
connection_type = "serial"

# --- Serial port (connection_type = "serial") ---
port_name = "/dev/ttyACM0"
baud_rate  = 9600
set_gps_to_10hz = false

# --- TCP bridge (connection_type = "tcp") ---
# tcp_host = "192.168.1.10"
# tcp_port = 9001

mqtt_enabled = true          # false = display-only, no MQTT publishing
mqtt_host    = "localhost"
mqtt_port    = 1883
# mqtt_client_id = "gps-to-mqtt"   # auto-generated when absent
mqtt_base_topic = "/GOLF86/GPS"
# mqtt_username = ""
# mqtt_password = ""
# mqtt_use_tls  = false

log_level = "info"           # trace | debug | info | warn | error
log_json  = false             # true = JSON structured logs

track_mode = "disabled"      # disabled | manual | learn | gpx
```

### TCP Bridge Mode

Instead of a locally attached serial GPS device you can connect to a
[io-to-net](https://github.com/askrejans/io-to-net) bridge that exposes the
serial port over TCP. This is useful for:

- GPS devices attached to a remote Raspberry Pi or embedded board
- Running gps-to-mqtt on a different machine than the one with the GPS dongle
- Sharing a single GPS device with multiple consumers

**gps-to-mqtt config (`settings.toml`):**

```toml
connection_type = "tcp"
tcp_host = "192.168.1.10"   # address of the io-to-net host
tcp_port = 9001
```

**io-to-net bridge config (on the host with the GPS dongle):**

```toml
[[bridges]]
name = "gps"
frame_mode = "line"   # each NMEA sentence arrives as one complete line

[bridges.source]
type  = "serial"
port  = "/dev/ttyUSB0"
baud  = 9600

[bridges.transport]
type   = "tcp"
listen = "[::]:9001"
```

The TCP task handles both line-framed and raw-stream bridges — data is
buffered internally and NMEA sentences are extracted whenever a `\n` is found,
regardless of TCP packet boundaries. Automatic reconnection with exponential
backoff is applied when the bridge connection drops.

### Prometheus Metrics

Enable a lightweight HTTP server that exposes a Prometheus scrape endpoint and a JSON health check:

```toml
prometheus_enabled = true
prometheus_port    = 9090
prometheus_bind    = "0.0.0.0"   # or "127.0.0.1" to restrict access
```

| Endpoint | Description |
|----------|-------------|
| `GET /metrics` | Prometheus text format (scrape target) |
| `GET /health` | JSON health summary |

**Exposed metrics:**

| Metric | Type | Description |
|--------|------|-------------|
| `gps_nmea_sentences_total` | counter | Total NMEA sentences received |
| `gps_connected` | gauge | 1 if GPS source is connected |
| `gps_fix_quality` | gauge | Fix quality (0=invalid, 1=GPS, 2=DGPS, 4=RTK…) |
| `gps_satellites_used` | gauge | Satellites used in current fix |
| `gps_satellites_in_view` | gauge | Total tracked satellites |
| `gps_hdop` | gauge | Horizontal dilution of precision |
| `gps_speed_kmh` | gauge | Current speed in km/h |
| `gps_altitude_meters` | gauge | Altitude above sea level (m) |
| `gps_position_accuracy_meters` | gauge | 2D position accuracy (m, from GST) |
| `mqtt_connected` | gauge | 1 if MQTT broker is connected |
| `mqtt_messages_published_total` | counter | Total MQTT messages published |

**Example Prometheus scrape config:**

```yaml
scrape_configs:
  - job_name: gps-to-mqtt
    static_configs:
      - targets: ["localhost:9090"]
```

**Health endpoint response:**

```json
{
  "status": "ok",
  "gps_connected": true,
  "mqtt_connected": true,
  "mqtt_enabled": true,
  "connection_address": "/dev/ttyUSB0 @ 9600 baud"
}
```

`status` is `"ok"` when GPS is connected, `"degraded"` otherwise.

### Environment Variables

All settings can be overridden via `GPS_TO_MQTT_*` environment variables
(highest priority — overrides any config file):

| Variable | Equivalent setting |
|----------|-------------------|
| `GPS_TO_MQTT_CONNECTION_TYPE` | `connection_type` |
| `GPS_TO_MQTT_PORT_NAME` | `port_name` |
| `GPS_TO_MQTT_BAUD_RATE` | `baud_rate` |
| `GPS_TO_MQTT_TCP_HOST` | `tcp_host` |
| `GPS_TO_MQTT_TCP_PORT` | `tcp_port` |
| `GPS_TO_MQTT_MQTT_ENABLED` | `mqtt_enabled` |
| `GPS_TO_MQTT_MQTT_HOST` | `mqtt_host` |
| `GPS_TO_MQTT_MQTT_PORT` | `mqtt_port` |
| `GPS_TO_MQTT_MQTT_BASE_TOPIC` | `mqtt_base_topic` |
| `GPS_TO_MQTT_MQTT_USERNAME` | `mqtt_username` |
| `GPS_TO_MQTT_MQTT_PASSWORD` | `mqtt_password` |
| `GPS_TO_MQTT_LOG_LEVEL` | `log_level` |
| `GPS_TO_MQTT_LOG_JSON` | `log_json` |
| `GPS_TO_MQTT_PROMETHEUS_ENABLED` | `prometheus_enabled` |
| `GPS_TO_MQTT_PROMETHEUS_PORT` | `prometheus_port` |
| `GPS_TO_MQTT_PROMETHEUS_BIND` | `prometheus_bind` |

## MQTT Topics

All topics are published under the configurable `mqtt_base_topic` prefix (default `/GOLF86/GPS`).  
Only values that have changed are republished (retained, QoS 0).

### Position

| Topic | Unit | Notes |
|-------|------|-------|
| `/LAT` | decimal degrees | Latitude |
| `/LNG` | decimal degrees | Longitude |
| `/ALT` | metres | Altitude above sea level |
| `/ALT_FT` | feet | Altitude above sea level |

### Speed & Course

| Topic | Unit | Notes |
|-------|------|-------|
| `/SPD` | km/h | Speed (alias for `/SPD_KPH`) |
| `/SPD_KPH` | km/h | Speed over ground |
| `/SPD_MPH` | mph | Speed over ground |
| `/SPD_KTS` | knots | Speed over ground |
| `/CRS` | degrees | Course over ground — suppressed below 3 km/h |

### Fix & Satellite Quality

| Topic | Unit / Values | Notes |
|-------|--------------|-------|
| `/SATS` | integer | Satellites used in current fix |
| `/SAT/GLOBAL/NUM` | integer | Total satellites tracked |
| `/HDOP` | — | Horizontal dilution of precision |
| `/VDOP` | — | Vertical dilution of precision |
| `/PDOP` | — | Position dilution of precision |
| `/QTY` | 0–8 | Fix quality: 0=Invalid, 1=GPS, 2=DGPS, 3=PPS, 4=RTK, 5=Float RTK, 6=Estimated, 7=Manual, 8=Simulation |
| `/TME` | HH:MM:SS | GPS UTC time |
| `/DTE` | DD.MM.YYYY | GPS UTC date |
| `/POSITION_ACCURACY` | metres | 2D position accuracy (from GST) |
| `/TRUE_HEADING` | degrees | True heading (from HDT) |
| `/HEADING_RATE_GPS` | deg/s | Heading rate from GPS ROT sentence |

### Per-Satellite

| Topic | Format | Notes |
|-------|--------|-------|
| `/SAT/VEHICLES/{prn}` | text | `PRN: N, Type: X, Elevation: N, Azimuth: N, SNR: N, In View: true/false` |

### Telemetry  *(requires `telemetry_enabled = true`)*

Derived values are always published; zero is used when speed is below the 3 km/h noise threshold.

| Topic | Unit | Notes |
|-------|------|-------|
| `/ACCEL_LONG_MPS2` | m/s² | Longitudinal acceleration |
| `/ACCEL_LONG_G` | g | Longitudinal acceleration |
| `/ACCEL_LAT_MPS2` | m/s² | Lateral (centripetal) acceleration |
| `/ACCEL_LAT_G` | g | Lateral acceleration |
| `/COMBINED_G` | g | √(long_g² + lat_g²) total g-load |
| `/HEADING_RATE` | deg/s | Yaw rate (ROT preferred, course-diff fallback) |
| `/DISTANCE` | metres | Cumulative distance this session |
| `/MAX_SPEED` | km/h | Session maximum speed |
| `/MAX_SPEED_MPH` | mph | Session maximum speed |
| `/BRAKING` | 0 / 1 | 1 when deceleration < −0.5 m/s² |

### Lap Timing  *(requires track mode enabled)*

| Topic | Unit | Notes |
|-------|------|-------|
| `/LAP_NUMBER` | integer | Current lap counter |
| `/LAP_TIME` | seconds | Last completed lap time |
| `/BEST_LAP` | seconds | Best lap time this session |
| `/SECTOR_1`, `/SECTOR_2`, … | seconds | Individual sector times |


### Compatibility and Testing

These packages have been tested on both Raspberry Pi 4 (ARM) with DEB packages and x86 systems with RPM packages. However, please note that this project is a work in progress, and more tests are needed, especially with real ECUs. Exercise caution when using, and stay tuned for updates as development continues to enhance and stabilize the functionality.

Feel free to reach out if you have any questions or encounter issues. Happy telemetry monitoring! 📊🛠️

## Architecture

The application uses a modern async architecture built on tokio:

```
Serial Port Task (blocking)  ┐
  OR                         ├─→ GPS Parser → Event Channel
TCP Bridge Task (async)      ┘                     ↓
                                          State Aggregator
                                                   ↓
                                             Shared State ← TUI/CLI/Service
                                                   ↓
                                            MQTT Publisher
```

### Key Components

- **parser.rs**: Pure NMEA sentence parser returning structured events
- **serial.rs**: Serial port handler with reconnection logic (blocking task)
- **tcp.rs**: Async TCP bridge handler with reconnection logic
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

### Run with Debug Logging

```bash
RUST_LOG=debug cargo run
```

### Run with a Custom Config

```bash
cargo run -- --config /path/to/settings.toml
```

### Cross-Compilation

The build script uses `cross` with `musl` targets for fully-static binaries:

- `x86_64-unknown-linux-musl` (x86-64)
- `aarch64-unknown-linux-musl` (arm64 / Raspberry Pi 4+)

```bash
# Manual cross-build example
cross build --release --target aarch64-unknown-linux-musl
```

## Troubleshooting

### Serial Port Access

If you get permission errors:

```bash
# Add your user to the dialout group
sudo usermod -a -G dialout $USER
# Log out and back in for changes to take effect
```

### MQTT Connection Issues

- Verify the MQTT broker is running: `mosquitto_sub -h localhost -t '#' -v`
- Check firewall rules if connecting to a remote broker
- Enable debug logging: `--log-level debug`

### TUI Display Issues

- Ensure your terminal supports UTF-8 and has at least 80x24 character dimensions
- Try a different terminal emulator if rendering issues occur

## License

This project is licensed under the [MIT License](LICENSE). Feel free to use, modify, and distribute the code as per the license terms.

## Contributing

Contributions are welcome! Please submit issues and pull requests on GitHub.
