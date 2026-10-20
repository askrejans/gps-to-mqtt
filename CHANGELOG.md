# Changelog

## 0.7.1

### Added
- On-demand position for consumers on slow links. Publish a reply topic to
  `<base>/position/get` to receive the latest valid fix younger than 2 s on it
  (QoS 0, not retained): position, speed, heading, lateral and longitudinal G,
  and a `channels` object with every other numeric value (altitude, accuracy,
  satellites, DOPs, combined G, yaw rate, distance, max speed). The consumer asks
  again after each answer, so it never builds a backlog. Answers are rate limited
  to one per 10 ms per reply topic, for up to 32 reply topics. Used by
  G86-ESP-DSP 0.2.

## 0.7.0

### Contract changes
- Licence changed to GPL-3.0-only.
- Lap timing removed (`track_*` settings and `LAP_*` / `BEST_LAP` / `SECTOR_*` topics).
- Legacy `SAT/VEHICLES/<id>` topics are keyed by constellation (`G07`, `R65`, `E03`).
  The payload format is unchanged. Retained topics of satellites that leave the sky are cleared.
- Legacy `LAT`/`LNG`/`ALT` are published only for valid fixes; `SPD_KTS` uses two decimals.
- HTTP server binds to `127.0.0.1` by default; `/health` returns 503 without a fresh valid fix.
- Defaults: `port_name = "auto"`, `baud_rate = 0` (auto-detect). Explicit values behave as before.
- The G86 `<base>/position` packet (schema 1) is unchanged.

### Added
- Sources: UDP, gpsd client, NMEA file replay (`--replay`), serial auto-detection of port and baud.
- Receiver profiles `ublox`, `mediatek`, `quectel` with `update_rate_hz` (1–25 Hz):
  sentence trimming, automatic 115200 baud with verification and fallback. `init_commands`.
- Epoch engine: one coherent fix per navigation epoch; GNS, GST, ZDA, THS, PUBX,00;
  NMEA 4.10+ system IDs; QZSS, NavIC, SBAS; accuracy provenance (GST / receiver / HDOP estimate).
- MGRS 10-digit (1 m) and 8-digit (10 m) references in JSON, API, MQTT (`/MGRS`, `/MGRS_8`),
  Home Assistant, terminal and web dashboards.
- Static hold (`static_hold`): stable position while stationary, flagged as `position_held`.
- MQTT formats: `json` (`/fix`, `/sky`), `homeassistant` discovery, `owntracks`; `<base>/status`
  with last will; QoS, retain and rate limit settings; TLS CA and client certificates; `mqtt_password_file`.
- HTTP API: `/api/v1/{status,fix,sky,geojson,nmea,config,stream}` (SSE), bearer token, CORS option.
- Built-in web dashboard with Drive, Navigation (dark OpenStreetMap track), Satellites, System and
  Configuration modes.
- NMEA-0183 TCP server and UDP sender; gpsd-compatible JSON server.
- CLI: `--check-config`, `--list-ports [--probe]`, `--headless`, `--healthcheck`.
- Terminal dashboard: status bar, g-g diagram, satellite table, statistics tab, raw view pause.

### Fixed
- Multi-word `GPS_TO_MQTT_*` environment variables were ignored.
- Telemetry was computed per NMEA event with host time; it is now per fix with receiver time.
- `set_gps_to_10hz` did not deliver complete 10 Hz data (link bandwidth); now handled by the u-blox plan.
- A crafted non-ASCII checksum could crash the TCP reader.
- `$GPTXT` messages mentioning other sentence names were misparsed; satellites collided across constellations.
- Health stayed "ok" after the receiver was unplugged.
- Trailing slash in `mqtt_base_topic` produced `//` topics; fixed MQTT client ID clashes.
- State lock held across archive `fsync`; full state cloned per event into the MQTT queue.
- Terminal not restored after a panic; ANSI colours in journald; lost early log lines.

### Security
- Dependencies updated; 20 `cargo audit` advisories resolved (rustls, rustls-webpki,
  aws-lc-sys, bytes, quick-xml, atty, gumdrop). One documented exception
  (`.cargo/audit.toml`, rumqttc's unused webpki 0.102 error type).
- Bounded framing for all inputs, client limits, strict CSP for the dashboard,
  secrets masked in `--check-config` and `/api/v1/config`.

## 0.6.0
- Coherent external GPS packets for G86 and a durable local archive.
