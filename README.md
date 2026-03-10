# GPS-to-MQTT

A Rust application that bridges GPS hardware to MQTT-based systems, enabling real-time GPS data integration into IoT and telemetry applications. It reads NMEA-0183 format data from USB GPS dongles or a TCP bridge, processes the various sentence types, and publishes parsed information to configurable MQTT topics. Supports an interactive terminal UI for standalone/bench use, full racing telemetry, and fully optional MQTT so the app can run display-only without any broker.

![gps](https://github.com/user-attachments/assets/14f42bc3-59a6-4973-99da-32b818c8b44e)

### Key Capabilities

- **GPS Data Processing** – Reads and parses standard NMEA-0183 sentences including position, speed, course, heading, accuracy and satellite information
- **Dual Input Modes** – Connect via a local serial port **or** a TCP bridge (e.g. [io-to-net](https://github.com/askrejans/io-to-net)) — configurable with a single setting
- **Real-time MQTT Publishing** – Converts GPS data into structured MQTT messages with configurable topics and QoS levels
- **High-Frequency Updates** – Optional 10 Hz update rate on compatible u-blox GPS modules
- **Flexible Configuration** – TOML config file, environment variables with `GPS_TO_MQTT_` prefix, and automatic `.env` file loading from the working directory
- **Automatic Mode Detection** – Runs the interactive TUI when attached to a terminal; falls back to structured service logging when run as a daemon
- **Racing Telemetry** – Real-time acceleration, g-forces, yaw rate, lap timing with sector support, braking detection and distance tracking
- **Prometheus Metrics** – Optional HTTP metrics endpoint and JSON health check for observability

### Hardware Compatibility

The software supports standard NMEA-0183 protocols and has been primarily tested with the **TOPGNSS GN800G GPS module** (M8030-KT chipset). The 10 Hz high-frequency mode specifically targets u-blox compatible devices — use it at your own risk on untested hardware.

### Use Cases

- Car racing telemetry and lap timing
- Vehicle tracking and fleet management
- IoT sensor data collection
- Navigation and telemetry system integration

> **Note**: This is an ongoing development project. Contributions and feedback are welcome to improve compatibility and features.

## Features

- 📡 Reads NMEA-0183 GPS data from USB GPS dongles or a TCP bridge
- 🔄 Support for 10 Hz GPS update rate (u-blox devices only)
- 🛰️ Parses multiple NMEA sentence types (GSV, GGA, RMC, VTG, GSA, GLL, TXT, GNS, GST, ROT, HDT, PUBX)
- 🏁 Racing telemetry: acceleration, g-forces, lap timing, distance tracking, braking detection
- 📊 Publishes parsed data to MQTT topics (retained, QoS 0)
- 🖥️ Interactive TUI with satellite sky-view, live data panels and scrolling log
- 🌍 Multi-GNSS support (GPS, GLONASS, Galileo, BeiDou)
- 📈 Optional Prometheus metrics + HTTP health endpoint

<img width="1324" height="794" alt="Screenshot 2026-03-10 at 22 24 14" src="https://github.com/user-attachments/assets/fb49af25-f786-4145-8d70-259087a7fa94" />

<img width="1324" height="794" alt="Screenshot 2026-03-10 at 22 24 40" src="https://github.com/user-attachments/assets/09581858-b5c7-4804-a70b-12da0a500f1b" />


### ⚠️ 10 Hz Mode

Setting `set_gps_to_10hz = true` sends **proprietary u-blox binary commands** (including undocumented ones) directly to the GPS module to switch its update rate from 1 Hz to 10 Hz. This gives significantly smoother telemetry data — position, speed and heading update ten times per second instead of once.

**Use at your own risk.** These commands target u-blox chipsets (e.g. the M8030-KT). Sending them to an incompatible or unknown device may cause unexpected behaviour, incorrect data output, or require a device power-cycle to recover. The feature is disabled by default (`set_gps_to_10hz = false`) and should only be enabled if you know your hardware is u-blox compatible.

## Running modes

| Invocation | Behaviour |
|---|---|
| Terminal / bench (`ssh`, local shell) | Interactive TUI rendered via `ratatui` |
| `systemd` service / no TTY | Structured text logging to stdout |
| `mqtt_enabled = false` | No broker needed; data shown in TUI only |
| `mqtt_enabled = true` (default) | Data published to MQTT broker |

---

## Installation

Pre-built packages are available for all major platforms — no Rust toolchain needed.

### Debian / Ubuntu

```bash
curl -fsSL https://g86racing.com/packages/apt/gpg.key | sudo gpg --dearmor \
     -o /usr/share/keyrings/g86racing-archive-keyring.gpg

echo "deb [signed-by=/usr/share/keyrings/g86racing-archive-keyring.gpg] \
     https://g86racing.com/packages/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/g86racing.list

sudo apt update
sudo apt install gps-to-mqtt
```

### Fedora / RHEL / Rocky Linux

```bash
sudo tee /etc/yum.repos.d/g86racing.repo <<'EOF'
[g86racing]
name=G86Racing packages
baseurl=https://g86racing.com/packages/rpm
enabled=1
gpgcheck=0
EOF

sudo dnf install gps-to-mqtt
```

### macOS (Homebrew)

```bash
brew tap askrejans/g86racing
brew install gps-to-mqtt
```

To run as a background service (launchd):

```bash
brew services start askrejans/g86racing/gps-to-mqtt
```

Config is installed to `$(brew --prefix)/etc/gps-to-mqtt/settings.toml.example`.

### Windows

1. Download the latest `.zip` from [https://g86racing.com/packages/windows/](https://g86racing.com/packages/windows/).
2. Extract and copy `settings.toml.example` → `settings.toml`, then edit it.
3. Run interactively: `.\gps-to-mqtt.exe --config settings.toml`
4. Install as a Windows Service (optional, using [NSSM](https://nssm.cc)):

```powershell
nssm install gps-to-mqtt "C:\gps-to-mqtt\gps-to-mqtt.exe"
nssm set    gps-to-mqtt AppParameters "--config C:\gps-to-mqtt\settings.toml"
nssm start  gps-to-mqtt
```

### After Linux installation

```bash
sudo cp /etc/gps-to-mqtt/settings.toml.example /etc/gps-to-mqtt/settings.toml
sudo $EDITOR /etc/gps-to-mqtt/settings.toml
sudo systemctl start gps-to-mqtt
```

---

## Docker

The easiest way to run on any Linux machine or Raspberry Pi — no Rust toolchain needed.

### Quick start (serial GPS dongle)

```bash
# 1 – Clone / download the repo (or just grab docker-compose.yml)
git clone https://github.com/askrejans/gps-to-mqtt
cd gps-to-mqtt

# 2 – Edit the environment variables in docker-compose.yml
#     (GPS_TO_MQTT_MQTT_HOST, GPS_TO_MQTT_PORT_NAME, etc.)
$EDITOR docker-compose.yml

# 3 – Uncomment the serial device mapping:
#     devices:
#       - /dev/ttyUSB0:/dev/ttyUSB0

# 4 – Build and start
docker compose up -d

# Follow logs
docker compose logs -f
```

### Quick start (TCP bridge)

Set `GPS_TO_MQTT_CONNECTION_TYPE: tcp` and configure `GPS_TO_MQTT_TCP_HOST` / `GPS_TO_MQTT_TCP_PORT` in `docker-compose.yml`. No device mapping needed.

### docker-compose.yml reference

All configuration is done via **environment variables** in `docker-compose.yml` — no config file editing required.

| Variable | Default | Description |
|---|---|---|
| `GPS_TO_MQTT_CONNECTION_TYPE` | `serial` | `serial` or `tcp` |
| `GPS_TO_MQTT_PORT_NAME` | `/dev/ttyUSB0` | Serial device inside the container |
| `GPS_TO_MQTT_BAUD_RATE` | `9600` | Serial baud rate |
| `GPS_TO_MQTT_SET_GPS_TO_10HZ` | `false` | Enable 10 Hz mode (u-blox only) |
| `GPS_TO_MQTT_TCP_HOST` | — | TCP bridge hostname / IP |
| `GPS_TO_MQTT_TCP_PORT` | — | TCP bridge port |
| `GPS_TO_MQTT_MQTT_ENABLED` | `true` | Set `false` for display-only |
| `GPS_TO_MQTT_MQTT_HOST` | `localhost` | MQTT broker hostname |
| `GPS_TO_MQTT_MQTT_PORT` | `1883` | MQTT broker port |
| `GPS_TO_MQTT_MQTT_BASE_TOPIC` | `/CAR/GPS/` | Base MQTT topic prefix |
| `GPS_TO_MQTT_MQTT_USERNAME` | — | Broker username (optional) |
| `GPS_TO_MQTT_MQTT_PASSWORD` | — | Broker password (optional) |
| `GPS_TO_MQTT_LOG_LEVEL` | `info` | `trace` \| `debug` \| `info` \| `warn` \| `error` |
| `GPS_TO_MQTT_PROMETHEUS_ENABLED` | `false` | Enable Prometheus metrics endpoint |
| `GPS_TO_MQTT_PROMETHEUS_PORT` | `9090` | Prometheus metrics port |

### Using a settings.toml file instead

Mount your own config file over the default:

```yaml
# docker-compose.yml
volumes:
  - ./settings.toml:/etc/gps-to-mqtt/settings.toml:ro
```

Environment variables always take priority over the file, so you can use both.

### Serial port access on Linux

The container user is added to the `dialout` group. Ensure your host user can also access the device:

```bash
sudo usermod -aG dialout $USER   # then log out and back in
```

### Build the image yourself

```bash
docker build -t gps-to-mqtt .
```

---

## Building from source

### Quick start

```bash
# 1 – Build
cargo build --release

# 2 – Copy and edit config
cp example.settings.toml settings.toml
$EDITOR settings.toml

# 3 – Run (TUI auto-enabled when attached to a terminal)
./target/release/gps-to-mqtt

# 4 – Or pass a custom config path
./target/release/gps-to-mqtt --config /etc/gps-to-mqtt/settings.toml
```

## CLI options

```
Usage: gps-to-mqtt [options]

Options:
  -h, --help          Print help
  -c, --config FILE   Path to TOML config file (default: settings.toml)
```

---

## Configuration

Copy `example.settings.toml` to `settings.toml` and adjust the values. Every setting can also be set via an environment variable with the `GPS_TO_MQTT_` prefix.

```toml
# ── Connection ──────────────────────────────────────────────────
connection_type = "serial"   # "serial" | "tcp"

# Serial (used when connection_type = "serial")
port_name        = "/dev/ttyACM0"
baud_rate        = 9600
set_gps_to_10hz  = false     # u-blox only – use at your own risk

# TCP bridge (used when connection_type = "tcp")
# tcp_host = "192.168.1.10"
# tcp_port = 9001

# ── MQTT ────────────────────────────────────────────────────────
mqtt_enabled    = true
mqtt_host       = "localhost"
mqtt_port       = 1883
mqtt_base_topic = "/GOLF86/GPS"
# mqtt_username = ""
# mqtt_password = ""
# mqtt_use_tls  = false

# ── Logging ─────────────────────────────────────────────────────
log_level = "info"    # trace | debug | info | warn | error
log_json  = false     # true = JSON structured logs

# ── Prometheus metrics ──────────────────────────────────────────
# prometheus_enabled = false
# prometheus_port    = 9090
# prometheus_bind    = "0.0.0.0"

# ── Racing / track mode ─────────────────────────────────────────
# track_mode = "disabled"   # disabled | manual | learn | gpx
```

Key environment variables:

| Variable | Description |
|---|---|
| `GPS_TO_MQTT_CONNECTION_TYPE` | `serial` or `tcp` |
| `GPS_TO_MQTT_PORT_NAME` | Serial device path |
| `GPS_TO_MQTT_BAUD_RATE` | Serial baud rate |
| `GPS_TO_MQTT_TCP_HOST` / `GPS_TO_MQTT_TCP_PORT` | TCP bridge address |
| `GPS_TO_MQTT_MQTT_ENABLED` | `true` / `false` |
| `GPS_TO_MQTT_MQTT_HOST` / `GPS_TO_MQTT_MQTT_PORT` | Broker address |
| `GPS_TO_MQTT_MQTT_USERNAME` / `GPS_TO_MQTT_MQTT_PASSWORD` | Broker credentials |
| `GPS_TO_MQTT_LOG_LEVEL` | `trace` \| `debug` \| `info` \| `warn` \| `error` |
| `GPS_TO_MQTT_PROMETHEUS_ENABLED` | `true` / `false` |
| `GPS_TO_MQTT_PROMETHEUS_PORT` | Prometheus port |

### TCP Bridge Mode

Instead of a locally attached serial GPS device you can connect to a
[io-to-net](https://github.com/askrejans/io-to-net) bridge that exposes the
serial port over TCP.

```toml
connection_type = "tcp"
tcp_host = "192.168.1.10"
tcp_port = 9001
```

---

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

---

## Prometheus Metrics

Enable a lightweight HTTP server that exposes a Prometheus scrape endpoint and a JSON health check:

```toml
prometheus_enabled = true
prometheus_port    = 9090
prometheus_bind    = "0.0.0.0"
```

| Endpoint | Description |
|----------|-------------|
| `GET /metrics` | Prometheus text format (scrape target) |
| `GET /health` | JSON health summary |

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

---

## Building packages

The `scripts/build_packages.sh` script cross-compiles for **all platforms**.

### Targets

| Platform | Arch   | Rust triple                     | Output      |
|----------|--------|---------------------------------|-------------|
| Linux    | x64    | x86_64-unknown-linux-gnu        | .deb + .rpm |
| Linux    | arm64  | aarch64-unknown-linux-gnu       | .deb + .rpm |
| Windows  | x64    | x86_64-pc-windows-gnu           | .zip        |
| macOS    | x64    | x86_64-apple-darwin             | .tar.gz     |
| macOS    | arm64  | aarch64-apple-darwin            | .tar.gz     |

### Mac prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# cross – Docker-based cross-compiler for Linux + Windows targets
cargo install cross
# Docker Desktop must be running (and Rosetta enabled for Apple Silicon)

# macOS SDK (already present if Xcode CLT is installed)
xcode-select --install

# macOS cross-arch targets (native cargo, no Docker needed)
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Apple Silicon: pre-install cross-compilation toolchains needed inside Docker
for TRIPLE in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-gnu; do
    rustup toolchain install stable-$TRIPLE --force-non-host --profile minimal
done

# DEB packaging tool
brew install dpkg

# RPM packaging tool
brew install rpm
```

### Build commands

```bash
# Everything – all platforms, all arches, all package types
./scripts/build_packages.sh

# Linux only (DEB + RPM, all arches)
./scripts/build_packages.sh --platform linux

# Single format / arch
./scripts/build_packages.sh --platform linux  --arch arm64 --type deb
./scripts/build_packages.sh --platform windows --arch x64
./scripts/build_packages.sh --platform mac     --arch arm64

# Use local cargo instead of cross (you must have all toolchains installed)
./scripts/build_packages.sh --no-cross

./scripts/build_packages.sh --help
```

Output layout:

```
release/<version>/
  linux/
    deb/   *.deb (amd64, arm64)
    rpm/   *.rpm (x86_64, aarch64)
  windows/
         *.zip (x64)
  mac/
         *.tar.gz (x86_64, arm64)
         sha256sums.txt
```

Each Linux package:
- Installs the binary to `/usr/bin/gps-to-mqtt`
- Installs the systemd unit to `/lib/systemd/system/gps-to-mqtt.service`
- Drops an example config at `/etc/gps-to-mqtt/settings.toml.example`
- Creates a `gps` system user (in `dialout` group for serial GPS access)
- Enables the service on install

---

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

### Key components

- `src/config.rs` – configuration loading (TOML + env vars)
- `src/parser.rs` – NMEA sentence parser
- `src/telemetry.rs` – racing telemetry calculations
- `src/track.rs` – lap timing and track management
- `src/mqtt.rs` – async MQTT client with reconnection
- `src/serial.rs` – serial port handler with reconnection
- `src/tcp.rs` – async TCP bridge handler with reconnection
- `src/metrics.rs` – Prometheus metrics exposition
- `src/models.rs` – GPS and telemetry data structures
- `src/ui/` – ratatui TUI with multiple tabs
- `src/service.rs` – signal handling for graceful shutdown
- `src/logging.rs` – mode-specific logging configuration

---

## Troubleshooting

### Serial port access

```bash
sudo usermod -a -G dialout $USER
# Log out and back in for changes to take effect
```

### MQTT connection issues

```bash
mosquitto_sub -h localhost -t '#' -v
```

Enable debug logging: set `log_level = "debug"` in `settings.toml` or `GPS_TO_MQTT_LOG_LEVEL=debug`.

### TUI display issues

Ensure your terminal supports UTF-8 and has at least 80×24 characters.

---

## Related projects

- [Speeduino-to-MQTT](https://github.com/askrejans/speeduino-to-mqtt) – companion ECU data bridge
- [io-to-net](https://github.com/askrejans/io-to-net) – TCP bridge for serial GPS over network
- [G86 Web Dashboard](https://github.com/askrejans/G86-web-dashboard) – web dashboard for MQTT telemetry data

---

## License

This project is licensed under the [MIT License](LICENSE).
