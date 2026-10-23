# gps-to-mqtt

Plug-and-play GNSS gateway. Connect any NMEA-0183 receiver (USB, serial, TCP
bridge, UDP or gpsd) and publish coherent, timestamped fixes to MQTT, a REST /
SSE API with a built-in web dashboard, Prometheus, NMEA-0183 TCP/UDP and a
gpsd-compatible server. Written in Rust; runs on a Raspberry Pi, in Docker,
as a systemd service or interactively with a terminal dashboard.

![gps](https://github.com/user-attachments/assets/14f42bc3-59a6-4973-99da-32b818c8b44e)

## Standalone MQTT use

This bridge can publish to any MQTT broker. Configure the broker host, credentials and `mqtt_base_topic` for your own installation; use TLS for a broker reached across an untrusted network. The source distribution and ordinary binaries have no account, subscription or activation dependency. Product-specific subscription checks belong to separately built managed distributions.

For a custom namespace, set `mqtt_base_topic` explicitly (for example `/telemetry/car-1/gps`). An explicit prefix takes precedence over `car`. The optional legacy `car` shortcut keeps its existing `/g86/<car>/…` namespace for compatibility; omitting it does not create an account or select a vehicle profile. Existing defaults and packet fields remain compatible.

## Highlights

- **Zero configuration for USB receivers.** `port_name = "auto"` finds the receiver
  (u-blox, MediaTek, Quectel, Garmin, PL2303, CP210x, CH34x, FTDI …) and
  auto-detects the baud rate. Re-plugging is handled.
- **Any constellation, any NMEA version.** GPS, GLONASS, Galileo, BeiDou, QZSS,
  NavIC, SBAS; NMEA 2.x – 4.11. Strict checksums and bounded parsing.
- **Correct high rates.** u-blox/MediaTek profiles set 1–25 Hz *and* make it fit
  the link: trimmed sentence set, automatic 115200 baud, verified with fallback.
  Measured: a true 10 Hz with RMC+GGA every epoch on a u-blox M8030.
- **One fix per epoch.** RMC/GGA/GNS grouped by receiver time; derived dynamics
  (g-forces, yaw, distance, braking) use receiver time, not arrival time.
- **MGRS** 10-digit (1 m) and 8-digit (10 m) military grid references in every output.
- **Static hold** (optional): pins the position while parked, so idle jitter doesn't move coordinates.
- **Industry-standard outputs.** MQTT (scalar topics, JSON, Home Assistant
  discovery, OwnTracks, G86 app packet), HTTP REST/SSE/GeoJSON, Prometheus,
  NMEA-0183 over TCP/UDP (port 10110), gpsd JSON (port 2947).
- **Web dashboard**: Drive, Navigation (dark OpenStreetMap track), Satellites, System and Configuration modes.
- **Resilient.** Slow or offline outputs never block acquisition. Reconnects
  with backoff; health reflects real fix freshness.

## Quick start

```bash
cargo build --release
./target/release/gps-to-mqtt
```

On a terminal this opens the dashboard and starts looking for a receiver.
Without a config file it publishes to an MQTT broker on `localhost:1883`.

```bash
# Which port is my receiver on?
gps-to-mqtt --list-ports --probe

# Validate a config without starting
gps-to-mqtt --check-config -c settings.toml

# Bench test without hardware
gps-to-mqtt --replay tests/fixtures/ublox-m8-3d-fix-1hz.nmea
```

## Installation

Pre-built packages, no Rust toolchain needed.

**Debian / Ubuntu**

```bash
curl -fsSL https://g86racing.com/packages/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/g86racing-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/g86racing-archive-keyring.gpg] https://g86racing.com/packages/apt stable main" | sudo tee /etc/apt/sources.list.d/g86racing.list
sudo apt update && sudo apt install gps-to-mqtt
```

**Fedora / RHEL / Rocky**

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

**macOS (Homebrew)**

```bash
brew tap askrejans/g86racing
brew trust askrejans/g86racing        # Homebrew asks once for third-party taps
brew install gps-to-mqtt
brew services start askrejans/g86racing/gps-to-mqtt   # optional background service
```

**Windows**: download the `.zip` from <https://g86racing.com/packages/windows/>,
copy `settings.toml.example` to `settings.toml`, run
`gps-to-mqtt.exe --config settings.toml`. To run as a service use
[NSSM](https://nssm.cc) (`nssm install gps-to-mqtt …`).

**After a Linux package install**

```bash
sudo cp /etc/gps-to-mqtt/settings.toml.example /etc/gps-to-mqtt/settings.toml
sudo $EDITOR /etc/gps-to-mqtt/settings.toml
sudo systemctl restart gps-to-mqtt
journalctl -u gps-to-mqtt -f
```

Packages install the binary, a hardened systemd unit, the example config and
the licence, and create a `gps` service user in `dialout`.

## Sources

| `connection_type` | Use for | Key settings |
|-------------------|---------|--------------|
| `serial` (default) | USB dongles, UART, Raspberry Pi GNSS HATs | `port_name = "auto"` or a path, `baud_rate = 0` (auto) |
| `tcp` | Serial-to-network bridges ([io-to-net](https://github.com/askrejans/io-to-net), ser2net, Wi-Fi GNSS modules) | `tcp_host`, `tcp_port` |
| `udp` | Marine multiplexers, phone apps, OpenCPN | `udp_bind = "0.0.0.0:10110"` |
| `gpsd` | Share a receiver already managed by gpsd | `tcp_host`/`tcp_port` (default `127.0.0.1:2947`) |
| `file` | Replay recorded NMEA (tests, demos) | `replay_file`, `replay_speed`, `replay_loop` |

## Receivers

Every NMEA-0183 receiver works with the default `receiver = "generic"`, which
never writes to the device. Set a profile only to change the update rate:

| `receiver` | Devices | What `update_rate_hz` does |
|------------|---------|----------------------------|
| `ublox` | u-blox 6/7/M8/M9/M10/F9 | UBX-CFG-RATE; above 1 Hz trims GLL/VTG, GSA/GSV once per second, raises the port to 115200 |
| `mediatek` | MT33xx: GlobalTop PA6H, Adafruit Ultimate GPS, Quectel L76/L80 | PMTK220 + PMTK314 + PMTK251 |
| `quectel` | LC29H, LC79H, LG69T | PAIR050 |

Settings go to receiver RAM only, and a power cycle restores the stored
configuration. Anything else can be sent with `init_commands` (NMEA bodies
get `$` and a checksum automatically; `hex:` sends raw bytes):

```toml
receiver       = "ublox"
update_rate_hz = 10
init_commands  = ["PUBX,40,GLL,0,0,0,0"]
```

## Outputs

### MQTT

**Base topic** (`<base>` below), first match wins:

| Setting | `<base>` | Use |
|---------|----------|-----|
| `mqtt_base_topic = "/CAR/GPS"` (or `GPS_TO_MQTT_MQTT_BASE_TOPIC`) | `/CAR/GPS` | explicit, as before |
| `car = "golf86"` (or `GPS_TO_MQTT_CAR`) | `/g86/golf86/gps` | G86 car computer namespace (`/g86/<car>/…`); g86-logger sets it |
| neither | `/GOLF86/GPS` | legacy default, unchanged for existing installs |

`car` is a slug: `^[a-z0-9][a-z0-9-]{0,31}$`. The resolved base is logged at start.

Select any combination with `mqtt_formats`:

| Format | Topics | Notes |
|--------|--------|-------|
| `legacy` | `<base>/LAT`, `/LNG`, `/MGRS`, `/MGRS_8`, `/ALT`, `/SPD`, `/SPD_KPH`, `/SPD_MPH`, `/SPD_KTS`, `/CRS`, `/SATS`, `/HDOP`, `/VDOP`, `/PDOP`, `/QTY`, `/TME`, `/DTE`, `/SAT/GLOBAL/NUM`, `/SAT/VEHICLES/<G07…>`, telemetry `/ACCEL_*`, `/COMBINED_G`, `/HEADING_RATE`, `/DISTANCE`, `/MAX_SPEED`, `/BRAKING` | One retained value per topic, published only on change |
| `json` | `<base>/fix`, `<base>/sky` | Full fix (explicit units: `speed_mps`, `altitude_m`, `accuracy_m` + `accuracy_source`, `mgrs`, `mgrs_8`, `position_held`, `telemetry`) and satellites |
| `canonical` (`g86` accepted) | `<base>/position` | canonical external-GPS packet (schema 1), never retained, never throttled; `channels` carries `gpsSpeedKmh`, `altitudeM`, `accuracyM`, `hdop`, `satellitesUsed` |
| `homeassistant` | `homeassistant/device_tracker/…`, `homeassistant/sensor/…` | Discovery: location tracker plus speed, altitude, satellites, HDOP, accuracy, fix quality/mode, MGRS |
| `owntracks` | `owntracks/gps-to-mqtt/<base>` | `_type: location` for Home Assistant, Traccar, OwnTracks Recorder |

**On demand.** A consumer on a slow link (the G86-ESP-DSP display) publishes
its own reply topic as the payload to `<base>/position/get`. gps-to-mqtt
answers once on that topic with the latest valid fix younger than 2 s (QoS 0,
not retained; no answer without one):
`{"schema":1,"source":"gps-to-mqtt","timestampMs","latitude","longitude","speedMps","headingDeg","lateralG","longitudinalG","channels":{…}}`.
`channels` holds every other numeric value of the fix in canonical names:
`gpsSpeedKmh`, `altitudeM`, `accuracyM`, `satellitesUsed`, `satellitesInView`,
`hdop`, `vdop`, `pdop`, `rateOfTurnDps`, `combinedG`, `yawRateDps`,
`distanceM`, `maxSpeedKmh`. Missing values are left out. The consumer asks
again after using the answer, so no backlog builds up. Each request is answered with a sample the consumer hasn't had yet: when
there is nothing new, the request is held until the next sample (at most
2 s). A consumer can keep several requests in flight (the G86 displays keep
10) and gets every new sample once, as soon as it exists, without duplicates.
Per reply topic, answers are limited by a token bucket (bursts of 12, 100 a
second on average), for up to 32 reply topics.

`<base>/status` is `online`/`offline` (retained, with a last will).
`<base>/health` (retained JSON, every 5 s and on change) is the bridge health of
the G86 car computer: `state` (`ok` / `degraded` / `down`), receiver `link`,
position `rateHz` and `ageMs`, fresh `channels`, `errors` (`checksum`,
`malformed`, `reconnects`, `archive`, cumulative), `clockSynced`, `fix`:

```json
{"schema":1,"bridge":"gps-to-mqtt","version":"0.7.3","bootId":"4121-1792664120000000",
 "timestampMs":1792664130000,"state":"ok","link":"connected","sources":["gps-to-mqtt"],
 "rateHz":10.0,"ageMs":42,"channels":["accuracyM","altitudeM","gpsSpeedKmh","hdop","satellitesUsed"],
 "errors":{"archive":0,"checksum":0,"malformed":0,"reconnects":0},"clockSynced":true,"fix":true}
```

`down`: receiver link not connected or no sentence for 10 s. `degraded`: no
valid fix, last position older than 2 s, or an error counter rose in the last
5 s. Consumers treat a health message older than 15 s as down. Use
`mqtt_max_rate_hz` to throttle state topics on slow links. TLS: `mqtt_use_tls`,
`mqtt_ca_file`, and `mqtt_client_cert_file` + `mqtt_client_key_file` for
mutual TLS. Use `mqtt_password_file` for Docker/systemd secrets.

Example `fix` document:

```json
{"sequence":1042,"time":"2026-10-20T08:40:53Z","valid":true,"mode":"3d","quality":"gps",
 "latitude":56.9636182,"longitude":24.22893,"altitude_m":80.1,"speed_mps":0.09,
 "satellites_used":11,"satellites_in_view":22,"hdop":1.11,"accuracy_m":5.6,
 "accuracy_source":"hdop-estimate","mgrs":"35V LD 31524 16752","mgrs_8":"35V LD 3152 1675",
 "position_held":false,"telemetry":{"distance_m":0.0,"max_speed_mps":3.05,"braking":false}}
```

### HTTP API and web dashboard

```toml
http_enabled = true
http_bind    = "127.0.0.1"   # "0.0.0.0" for the network (then set http_token)
http_port    = 9090
http_token   = "change-me"   # protects /api/*
```

| Endpoint | |
|----------|-|
| `/` | Web dashboard |
| `/health` | 200 when the source is connected and a valid fix is ≤ 5 s old, otherwise 503 (`no_fix`, `source_down`) |
| `/metrics` | Prometheus |
| `/api/v1/status` · `/fix` · `/sky` · `/geojson` · `/nmea` · `/config` | JSON snapshots (`/config` is masked and read-only) |
| `/api/v1/stream` | Server-Sent Events: `fix`, `sky`, `position` |

The dashboard has five modes (keys `1`–`5`):

- **Drive**: g-g friction circle, g meters, yaw, braking, max speed, distance, compass.
- **Navigation**: decimal/DMS/MGRS position, heights, speeds, quality, and the
  track on a dark OpenStreetMap map (or a local-metre plot).
- **Satellites**: sky map, signal-strength bars, DOP, satellite table.
- **System**: link states, counters with rates, receiver messages, live NMEA monitor.
- **Configuration**: the effective settings and wiring.

It works with the token, uses a strict Content-Security-Policy, and sends no
requests except to the bridge and the configured map tile server. Set
`http_map_tiles = ""` for fully offline use.

### Network re-broadcast

```toml
nmea_tcp_listen = "0.0.0.0:10110"          # OpenCPN, Signal K, chart plotters
nmea_udp_target = "255.255.255.255:10110"  # broadcast to the LAN
gpsd_listen     = "127.0.0.1:2947"         # cgps, gpsmon, Navit, libgps clients
```

### Local archive

`telemetry_log = "/var/lib/gps-to-mqtt/telemetry.ndjson"` appends every G86
position packet and syncs it to disk on a dedicated writer, independent of MQTT.
Files are created with mode 0600 and capped at 512 MiB per active file. Rotate
by renaming (logrotate with `nocreate`, no `copytruncate`). Write failures are
counted in `archive_errors_total` and logged at most once a minute.

## Configuration

All settings are flat keys in `settings.toml` and can be overridden with
`GPS_TO_MQTT_<KEY>` environment variables (or a `.env` file):

```bash
GPS_TO_MQTT_MQTT_HOST=broker.lan GPS_TO_MQTT_MQTT_FORMATS=legacy,homeassistant gps-to-mqtt
```

Config files are searched in `/etc/gps-to-mqtt/settings.toml`, next to the
binary, and in the working directory, or given with `-c`. Every key is
documented in [example.settings.toml](example.settings.toml).

| Area | Keys |
|------|------|
| Source | `connection_type`, `port_name`, `baud_rate`, `tcp_host`, `tcp_port`, `udp_bind`, `replay_*`, `nmea_require_checksum` |
| Receiver | `receiver`, `update_rate_hz`, `init_commands` |
| MQTT | `mqtt_enabled`, `mqtt_host`, `mqtt_port`, `mqtt_base_topic`, `mqtt_formats`, `mqtt_qos`, `mqtt_retain`, `mqtt_max_rate_hz`, `mqtt_username`, `mqtt_password(_file)`, `mqtt_use_tls`, `mqtt_ca_file`, `mqtt_client_*_file` |
| HTTP | `http_enabled`, `http_bind`, `http_port`, `http_token`, `http_cors_origin`, `http_ui`, `http_map_tiles`, `http_map_dark` |
| Network | `nmea_tcp_listen`, `nmea_udp_target`, `gpsd_listen`, `max_clients` |
| Processing | `telemetry_enabled`, `telemetry_smoothing_window`, `static_hold`, `static_hold_speed_kph`, `static_hold_radius_m`, `satellite_timeout_s`, `telemetry_log` |
| Logging | `log_level`, `log_json`, `tui_refresh_rate_ms`, `max_log_buffer_size` |

Upgrading from 0.6: `prometheus_*` and `set_gps_to_10hz` are still accepted;
lap timing was removed. Note that environment variables now actually apply
(0.6 silently ignored multi-word variables). See [TECHNICAL.md](TECHNICAL.md#8-migrating-from-06).

## Docker

```bash
docker compose up -d        # edit environment in docker-compose.yml first
docker compose logs -f
```

The image runs as a non-root user with a read-only root filesystem in the
compose example. Map the receiver with `devices:` (prefer
`/dev/serial/by-id/…`). `HEALTHCHECK` uses `gps-to-mqtt --healthcheck`.
Build it yourself with `docker build -t gps-to-mqtt .`.

## Running modes

| Invocation | Behaviour |
|------------|-----------|
| Terminal | Interactive dashboard: overview, satellites + sky view, stats, logs, raw NMEA (`1`–`5`, `p` pause, `q` quit) |
| systemd / Docker / pipe, or `--headless` | Structured logs (`log_json = true` for JSON) |
| `mqtt_enabled = false` | Display/API only, no broker needed |

## CLI

```
gps-to-mqtt [OPTIONS]
  -c, --config <FILE>   Config file
      --check-config    Validate, print effective settings (secrets masked), exit
      --list-ports      List serial ports ranked by GNSS likelihood
      --probe           With --list-ports: probe each port for NMEA
      --replay <FILE>   Replay an NMEA log
      --headless        Never start the terminal dashboard
      --healthcheck     Exit 0 if /health is OK (container probes)
  -V, --version / -h, --help
```

## Troubleshooting

| Symptom | Check |
|---------|-------|
| `no NMEA receiver found` | `gps-to-mqtt --list-ports --probe`; Linux permissions: `sudo usermod -aG dialout $USER` and log in again |
| Fixes slower than configured | Set `receiver` to the right profile; over TCP the bridge's serial port must run at ≥ 115200 for 5–10 Hz |
| `/health` returns 503 `no_fix` | No sky view (indoors) or the receiver is still starting; see the Satellites mode |
| MQTT messages dropped | `mqtt_messages_dropped_total` rises while the broker is down; the state is re-sent on reconnect |
| Nothing on MQTT | `mosquitto_sub -h <broker> -t '<base>/#' -v`; `log_level = "debug"` |
| Garbled terminal dashboard | UTF-8 terminal, at least 100×30 |

## Building packages

`scripts/build_packages.sh` cross-compiles DEB/RPM (x64, arm64), Windows ZIP
and macOS tarballs (`--platform`, `--arch`, `--type`, `--no-cross`; see
`--help`). It needs `cross` + Docker for Linux/Windows targets, `dpkg` and
`rpm` from Homebrew, and the macOS rustup targets. Output goes to
`release/<version>/`.

## Development

```bash
cargo test                                   # unit, integration and real-receiver fixture tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo audit                                  # policy in .cargo/audit.toml
```

Architecture, output contracts, receiver measurements and the security model
are documented in [TECHNICAL.md](TECHNICAL.md). AI-assisted development
guidance lives in [AGENTS.md](AGENTS.md) and [skills/](skills/).

## Related projects

- [io-to-net](https://github.com/askrejans/io-to-net): serial-to-TCP bridge
- [Speeduino-to-MQTT](https://github.com/askrejans/speeduino-to-mqtt): ECU data bridge
- [G86 Web Dashboard](https://github.com/askrejans/G86-web-dashboard): MQTT telemetry dashboard

## License

GPL-3.0-only. See [LICENSE](LICENSE).
