# gps-to-mqtt — technical review and design (0.7.0)

This document records the review of 0.6.0, the problems found, and the design
of the 0.7.0 rework. It is meant for maintainers and for anyone integrating
with the bridge's outputs.

## 1. Summary

0.7.0 turns a single-purpose NMEA → MQTT bridge into a receiver-agnostic GNSS
gateway:

* **Inputs:** serial (auto-detected port and baud), TCP bridge, UDP, gpsd, file replay.
* **Receivers:** any NMEA-0183 talker (GPS, GLONASS, Galileo, BeiDou, QZSS, NavIC,
  SBAS; NMEA 2.x – 4.11). Optional u-blox, MediaTek and Quectel profiles configure
  the update rate (1–25 Hz) correctly, including link bandwidth.
* **Outputs:** MQTT (legacy topics, JSON, G86 packet, Home Assistant discovery,
  OwnTracks), HTTP (health, Prometheus, REST, SSE, GeoJSON), a built-in web
  dashboard, NMEA-0183 TCP/UDP re-broadcast and a gpsd-compatible JSON server.
* **Licence:** GPL-3.0-only from 0.7.0.

Existing consumers keep working: the G86 dashboard's legacy topics keep their
names and formats, and the G86 app's `<base>/position` packet is byte-compatible.

## 2. Review of 0.6.0

Severity: **H** = wrong data, crash or exposure; **M** = degraded operation;
**L** = quality/maintainability.

### 2.1 Security

| # | Sev | Finding | Resolution |
|---|-----|---------|------------|
| S1 | H | `cargo audit`: 20 advisories in the lockfile, including the MQTT TLS stack (rustls 0.23.35, rustls-webpki 0.102/0.103.8, aws-lc-sys 0.35, bytes 1.11.0), quick-xml, and unsound/unmaintained `atty` and `gumdrop`. | Lockfile refreshed; `atty` → `std::io::IsTerminal`; `gumdrop` → `clap`; `quick-xml` and `prometheus` removed. Remaining: rumqttc 0.25.1 names `webpki::Error` from rustls-webpki 0.102.8. Verification runs on the patched 0.103 line, so the vulnerable code is not executed. Documented exception in `.cargo/audit.toml`. |
| S2 | H | Remote panic: the TCP reader's checksum check sliced `&declared[..2]` on a UTF-8 string. A peer sending `$X*€` crashed the process. | All input goes through `LineFramer`, which keeps only printable ASCII ≤ 256 bytes, then `Sentence::parse` with strict, non-panicking checks. Tests cover malformed and non-ASCII input. |
| S3 | M | Metrics/health bound to `0.0.0.0` by default with no authentication. | Default bind `127.0.0.1`; optional bearer token for `/api/*` (constant-time compare); warning when bound publicly without a token; `nosniff` / `no-store` headers; CORS only when configured. |
| S4 | M | MQTT password only inline in the config; TLS limited to default roots. | `mqtt_password_file` (Docker/systemd secrets), custom CA, mutual-TLS client certificates. `--check-config` redacts secrets. |
| S5 | L | Unbounded per-line buffering and no client limits on network inputs. | 256-byte line bound, 8 KiB reads, `max_clients` for TCP servers, 1 KiB gpsd command bound. |

### 2.2 Correctness

| # | Sev | Finding | Resolution |
|---|-----|---------|------------|
| C1 | H | Environment variables silently ignored: `Environment::separator("_")` turned `GPS_TO_MQTT_MQTT_HOST` into the nested key `mqtt.host`, which never reached the flat `mqtt_host`. Every multi-word variable failed, which covers all of them, so Docker deployments ran on defaults. | No key separator, plus a regression test for multi-word variables. |
| C2 | H | Telemetry (acceleration, g, yaw) computed on **every event**, including each GSV satellite, TXT and VTG, using host `Instant::now()`. The derivative window filled with duplicate samples a few ms apart, which diluted or spiked the values. | One computation per emitted fix, using receiver UTC time (integer ms) for Δt. Gap, stationary, invalid and dead-reckoning resets. |
| C3 | H | 10 Hz mode did not deliver 10 Hz. On a u-blox M8030 the receiver's NMEA output budget is limited by the UART1 baud (measured ≈ 960 B/s at 9600, even on native USB). At 10 Hz it kept RMC and dropped most GGA (3 GGA per 21 RMC), so coherent fixes and G86 packets were rare. | Receiver plans (§5): trim the sentence set, set the rate, raise the port to 115200 when RMC+GGA don't fit, reopen, verify, fall back. Measured after the fix: 10.1 fixes/s, RMC+GGA every epoch, G86 packets exactly 100 ms apart. |
| C4 | H | Health never went down: `serial_connected` was set on the first event and never cleared, so `/health` reported `ok` and `gps_connected 1` after the device was unplugged. | Sources report `LinkStatus`; the hub marks a silent source after 5 s; health requires a connected source **and** a valid fix younger than 5 s (HTTP 503 otherwise). |
| C5 | M | Sentence type detected with `contains()`: a `$GPTXT` mentioning "GGA" was parsed as GGA. The parser itself did not verify checksums. | Address field split into talker + formatter; checksum verified before any field is read. |
| C6 | M | Satellites keyed by PRN only: GPS 3, Galileo 3 and BeiDou 3 overwrote each other (NMEA 4.10+). Satellites were never pruned from state or MQTT. | Key = (constellation, PRN), classified from talker and PRN range. Expiry after `satellite_timeout_s`; retained MQTT topics of departed satellites are cleared. |
| C7 | M | Base topic with a trailing slash produced `//LAT` (the shipped `docker-compose.yml` used `/CAR/GPS/`). | Normalised base topic. |
| C8 | M | Fixed MQTT client ID `gps-to-mqtt`: two bridges on one broker kicked each other off in a loop. No availability or LWT. | Unique default client ID; `<base>/status` online/offline with a retained last will; retained state re-sent after reconnect. |
| C9 | M | Lap timing used arrival-time geofence entry (missed at speed, ignored GPS time), and GPX `<trkpt/>` points were dropped. | Feature removed in 0.7 (out of scope; lap timing lives in the G86 app). |
| C10 | L | Config warnings were logged before logging was initialised (lost); the TUI log ring ignored `max_log_buffer_size`; service logs contained ANSI colours in journald. | Fixed. |

### 2.3 Stability and performance

| # | Sev | Finding | Resolution |
|---|-----|---------|------------|
| P1 | H | The event processor held the state **write lock across the archive `fsync`**, stalling the TUI and metrics for the duration of disk I/O at sensor rate. | Archive runs on its own task with a bounded queue. Snapshot locks are std `RwLock`s, never held across `.await`. |
| P2 | H | Every event (≈ 20–40/s at 1 Hz, ≈ 200/s at 10 Hz) cloned the full `GpsData`, including 500 raw NMEA strings and the satellite map, into the MQTT channel. The publisher then re-evaluated every topic and every satellite. | One `Arc<Fix>` per epoch and one `Arc<Sky>` per second on a broadcast bus. Allocations per epoch dropped by roughly two orders of magnitude. |
| P3 | M | Publishing awaited the MQTT request channel. During an outage, rumqttc's queue filled with stale messages that were flushed on reconnect. | `try_publish` only while connected, drops counted (`mqtt_messages_dropped_total`), full retained state re-sent on reconnect. |
| P4 | M | Terminal not restored after a panic in the TUI. | Panic hook restores raw mode and the alternate screen. |
| P5 | L | `tokio = full`, `prometheus` with a global registry (test races), duplicated serial/TCP reader code. | Narrow tokio features; metrics rendered from atomic counters; single framer/parser path for all sources. |

## 3. Architecture

```text
 ┌──────────── sources (one task) ────────────┐
 │ serial (blocking thread, auto-detect)      │
 │ tcp · udp · gpsd · file replay (async)     │──bytes──┐
 └────────────────────────────────────────────┘          │ mpsc(256), back-pressure
                                                          ▼
                                    ┌──────────────── hub ────────────────┐
                                    │ LineFramer → Sentence (checksum)    │
                                    │   ├─ PositionEncoder (G86 packet) ──┼──► archive task (fsync)
                                    │   └─ Engine (epochs) → Fix, Sky      │
                                    │      └─ TelemetryCalculator          │
                                    └───────┬──────────────────┬──────────┘
                              broadcast bus │ (lossy, 1024)    │ snapshot (RwLock)
             ┌──────────────┬───────────────┼──────────┬───────┴─────┬────────────┐
             ▼              ▼               ▼          ▼             ▼            ▼
           MQTT       NMEA TCP/UDP     gpsd server   HTTP SSE      HTTP REST     TUI
```

**Back-pressure policy.** Sources → hub is bounded and back-pressured: the hub
does no I/O and keeps up at any realistic rate. Hub → outputs is a lossy
broadcast. A slow or disconnected output lags and skips (counted), and never
slows acquisition or other outputs. The archive is the exception that must not
silently lose data, so it has its own bounded queue and counts every packet it
cannot persist (`archive_errors_total`).

**Epoch model.** NMEA arrives as a burst per navigation epoch. `Engine` groups
timed sentences (RMC, GGA, GNS, GLL) by UTC time and emits a `Fix`:

1. as soon as RMC and GGA/GNS for the epoch are both present (lowest latency);
2. otherwise when the next epoch starts (receivers sending only one of them);
3. or after 1.5 s of silence (idle flush).

Untimed sentences (GSA, VTG, GST, HDT/THS, ROT, PUBX,00) update auxiliary
values that attach to the next fix and expire after 3 s. Freshness is judged
against the epoch's own input time, so an idle flush does not drop them.

**Units.** SI at rest (m, m/s, degrees). Conversions happen only at output
boundaries: legacy `km/h`/`mph`/`kn`, OwnTracks `km/h`, gpsd `m/s`.

**MGRS.** Every valid fix carries `mgrs` (10 digits, 1 m) and `mgrs_8`
(8 digits, 10 m), e.g. `35V LD 31524 16761`. The conversion uses the Snyder
transverse-Mercator series with the Norway (32V) and Svalbard (31/33/35/37X)
exceptions. Digits are truncated, as the military convention requires. It was
validated against an independent Krüger n-series implementation: within 5 cm
at every reference point, including 4° off-meridian in zone 32V. MGRS is
undefined in the polar UPS regions (north of 84°N, south of 80°S), where the
fields are `null`.

**Static hold** (`static_hold = true`). Below `static_hold_speed_kph` the
reported position is pinned to the running mean of the stationary samples.
The reported value only moves when that mean has moved ≥ 1 m. Speed is
reported as 0, course is suppressed and `position_held` is `true`. Hold
releases immediately when speed rises or the raw position leaves
`static_hold_radius_m`. Hold runs before MGRS and telemetry, so the odometer
does not accumulate jitter either. On the real capture, 12 distinct raw
positions became ≤ 3 reported ones. The G86 `/position` packet is built from
the raw sentences and is unaffected, because the app's contract expects
receiver fixes.

**Validity.** `Fix.valid` requires a non-void status (RMC/GLL `A`), a GGA/mode
quality other than invalid, and a position. Telemetry also requires a *measured*
quality (GPS/DGPS/PPS/RTK), so dead-reckoned and simulated fixes never produce
dynamics.

### Modules

| Path | Responsibility |
|------|----------------|
| `src/nmea/` | `LineFramer`, `Sentence` (checksum, talker, formatter), strict field parsers |
| `src/engine.rs` | Epoch aggregation, satellites, accuracy provenance |
| `src/telemetry.rs` | g-forces, yaw, odometer, braking (per fix, receiver time) |
| `src/mgrs.rs` | WGS-84 → UTM → MGRS (8/10-digit), zone exceptions |
| `src/hold.rs` | Static hold for stationary positions |
| `src/position.rs` | G86 external-GPS packet (contract; independent strict checks) |
| `src/receiver.rs` | UBX / PMTK / PAIR command builders, rate plans, custom commands |
| `src/source/` | Serial (detection, plan application), TCP, UDP, gpsd, replay |
| `src/hub.rs` | The single consumer of source bytes; fan-out |
| `src/shared.rs` | Bus, snapshot, counters, health |
| `src/output/` | MQTT, HTTP, web dashboard assets, NMEA TCP/UDP, gpsd |
| `web/` | Dashboard (HTML/CSS/JS, no build step; embedded with `include_str!`) |
| `src/archive.rs` | Synced NDJSON archive |
| `src/ui/` | ratatui dashboard |
| `src/config.rs` | Flat config, env mapping, validation |

## 4. Output contracts

### 4.1 MQTT

All topics are under the resolved base (trailing `/` removed): `mqtt_base_topic`
when set, else `/g86/<car>/gps` when `car` is set, else the legacy `/GOLF86/GPS`.

* **`<base>/status`**: `online` / `offline`, retained, QoS 1. `offline` is also the last will.
* **`<base>/health`**: retained JSON, QoS 1, every 5 s and at once when `state`
  or `link` changes; a final `down` before the clean-shutdown `offline`
  (G86 `contracts/CAR_COMPUTER.md` §2, `src/health.rs`). `rateHz`/`ageMs`/`channels`
  describe published `<base>/position` packets; `errors` are cumulative
  `checksum`, `malformed`, `reconnects` (receiver link losses), `archive`;
  `clockSynced` is the kernel NTP state (`adjtimex`) on Linux, `true` elsewhere.
* **`legacy`**: retained, change-detected scalar topics, names and number formats unchanged from 0.6:
  `LAT LNG MGRS MGRS_8 ALT ALT_FT SPD SPD_KPH SPD_MPH SPD_KTS CRS SATS SAT/GLOBAL/NUM HDOP VDOP PDOP QTY TME DTE POSITION_ACCURACY TRUE_HEADING HEADING_RATE_GPS`,
  telemetry `ACCEL_LONG_MPS2 ACCEL_LONG_G ACCEL_LAT_MPS2 ACCEL_LAT_G COMBINED_G HEADING_RATE DISTANCE MAX_SPEED MAX_SPEED_MPH BRAKING`,
  and `SAT/VEHICLES/<id>` with the 0.6 text payload
  `PRN: 7, Type: Gps, Elevation: 67, Azimuth: 182, SNR: 37, In View: true`.
  Changes: `<id>` is now constellation-prefixed (`G07`, `R65`, `E03`) to avoid
  PRN collisions; `LAT/LNG/ALT` are published only for valid fixes; `SPD_KTS`
  has two decimals; lap topics were removed.
* **`json`**: `<base>/fix` is the serialized `Fix` (field names carry units:
  `speed_mps`, `altitude_m`, `accuracy_m` + `accuracy_source`), and `<base>/sky`
  is satellites with `used` flags and DOPs.
* **`g86`**: `<base>/position`, never retained (schema 1, unchanged): `schema,
  source, bootId, sequence, timestampMs, latitude, longitude, speedMps,
  headingDeg, accuracyM, altitudeM, accuracySource` plus (0.7.3, additive)
  `channels: {gpsSpeedKmh, altitudeM, accuracyM, hdop, satellitesUsed}` in
  registry names. `source` stays `nmea` (released G86 apps require it); `bootId`
  is the same as on `<base>/health`. Produced only when RMC and
  GGA of the same epoch agree (time, position within 2e-5°), RMC is `A`, GGA
  quality is 1–5 and checksums match. Not throttled.
* **`homeassistant`**: retained discovery documents under
  `<prefix>/device_tracker/<node>/config` and `<prefix>/sensor/<node>/<key>/config`
  (speed, altitude, satellites, HDOP, accuracy, fix quality, fix mode, MGRS), all
  reading `<base>/fix` with `<base>/status` as availability. `<node>` is derived
  from the client ID or base topic and is stable across restarts.
* **`owntracks`**: `_type: location` with `lat, lon, tst, acc, alt, vel (km/h), cog, tid`.

### 4.2 HTTP

| Route | Notes |
|-------|-------|
| `GET /health` | 200 `{"status":"ok"}` with a connected source and a valid fix ≤ 5 s old; 503 with `no_fix` / `source_down` otherwise |
| `GET /metrics` | Prometheus text 0.0.4 |
| `GET /` | web dashboard (when `http_ui = true`, the default) |
| `GET /api/v1/status` | wiring, link states, rates, counters, receiver TXT messages |
| `GET /api/v1/config` | effective configuration, secrets masked |
| `GET /api/v1/fix` · `/sky` · `/geojson` | latest snapshot (`application/geo+json` Feature for GeoJSON) |
| `GET /api/v1/nmea?limit=N` | last N verified sentences (max 500) |
| `GET /api/v1/stream` | SSE events `fix`, `sky`, `position`; keep-alive 15 s |

Metrics: `gps_bytes_received_total, gps_nmea_sentences_total,
gps_checksum_errors_total, gps_malformed_sentences_total, gps_fixes_total,
gps_connected, gps_fix_valid, gps_fix_age_seconds, gps_fix_quality,
gps_satellites_used, gps_satellites_in_view, gps_hdop, gps_speed_kmh,
gps_altitude_meters, gps_position_accuracy_meters, gps_sentence_rate_hz,
gps_fix_rate_hz, mqtt_connected, mqtt_messages_published_total,
mqtt_messages_dropped_total, archive_errors_total, output_clients,
gps_to_mqtt_build_info`. All 0.6 metric names are preserved.

### 4.3 Network re-broadcast

* **NMEA-0183 TCP** (`nmea_tcp_listen`, standard port 10110): verified sentences, CRLF-terminated, verbatim.
* **NMEA-0183 UDP** (`nmea_udp_target`): one datagram per sentence; broadcast allowed.
* **gpsd** (`gpsd_listen`, protocol 3.15 subset): `VERSION` on connect;
  `?WATCH`, `?VERSION`, `?DEVICES`, `?POLL`; reports `TPV` (mode, status, time,
  lat/lon, altMSL/altHAE/geoidSep, speed, track, eph/epx/epy/epv, magvar,
  dgpsAge/Sta), `SKY` (PRN, gnssid, svid, el, az, ss, used, DOPs, nSat/uSat)
  and raw NMEA with `"nmea":true`.

### 4.4 Web dashboard

`GET /` serves a single-page dashboard embedded in the binary (`web/`). No
build step, no framework, no remote resources. It uses the same API as any
other client: the REST endpoints plus `api/v1/stream`, which it reads with
`fetch()` so the bearer token goes in a header (EventSource cannot set headers).
The token is entered once and kept in the browser's `localStorage`.

Modes (keys `1`–`5`, also addressable as `#drive`, `#nav`, `#sky`, `#system`, `#config`):

| Mode | Content |
|------|---------|
| Drive | g-g friction circle with 3 s trail, longitudinal/lateral/yaw centre-zero meters, braking, max speed, distance, peak g, course/heading compass |
| Navigation | position (decimal, DMS, MGRS 10- and 8-digit), MSL/ellipsoidal height, speed in all units, course/heading/magnetic variation, track on a dark OpenStreetMap layer (or local-metre plot) with accuracy ring and course, quality details (DOP, GST σ, DGPS, static hold), copy coordinates/MGRS, GeoJSON download |
| Satellites | polar sky map (constellation colour, filled = used), C/N₀ bar chart with 25/40 dB-Hz thresholds, DOP meters, satellite table |
| System | link states, counters with per-second rates, receiver TXT messages, live NMEA monitor with filter/pause |
| Configuration | effective configuration grouped by section (read-only, secrets masked), wiring, endpoints |

Persistent primary readout: speed, fix badge, satellites, accuracy, altitude,
course, fix rate. Status lamps: source, fix, MQTT, live stream. Unit toggle
(km/h · mph · kn) and a brightness toggle for night use.

Map: a small built-in Web-Mercator tile renderer, with no map library.
Tiles come from `http_map_tiles` (OpenStreetMap by default) and are drawn on a
lower canvas with a CSS invert/hue-rotate filter, so overlays stay true-colour.
Tiles are requested with an origin-only referrer, as OSM's tile policy expects;
the attribution is always shown; at most 400 tiles are cached in memory; zoom
is Ctrl/⌘ + wheel (a plain wheel scrolls the page) or the ± keys; drag pans and
stops follow mode. Viewing the map tells the tile server which area is shown.
Set `http_map_tiles = ""` for offline use or no third-party requests, and the
local plot takes over. The template is validated (fixed host, `{z}{x}{y}`,
URL characters only) because its origin is copied into the CSP header.

Security: assets are public, data is not (API token applies). The CSP is
`default-src 'none'` with only `'self'` for scripts, styles and connections
(plus the tile origin for images when the map is enabled),
plus `frame-ancestors 'none'`, `X-Frame-Options: DENY` and `no-referrer`. All
dynamic text is assigned with `textContent`, since receiver messages and NMEA
are untrusted. Configuration is read-only by design: changing receiver or
broker settings from a browser would be a remote-control surface.

## 5. Receiver support

Any NMEA-0183 source works with `receiver = "generic"`, which never writes to
the device. Profiles only matter when `update_rate_hz` is set:

| Profile | Rate | Above 1 Hz | Link speed |
|---------|------|------------|------------|
| `ublox` | UBX-CFG-RATE | UBX-CFG-MSG: GLL/VTG off, GSA/GSV once per second | UBX-CFG-PRT UART1 115200 when needed |
| `mediatek` | PMTK220 | PMTK314: RMC+GGA every fix, GSA/GSV every 5th | PMTK251,115200 when needed |
| `quectel` | PAIR050 | — | — (configure with `init_commands`) |

"When needed" means RMC+GGA (≈ 150 B/epoch) × rate plus the once-per-second
GSA/GSV burst (≈ 700 B) exceeds 80 % of the link budget. In practice that means
any rate above 1 Hz on a 9600 link. Counting only RMC+GGA was not enough: at
5 Hz the receiver then dropped every GSV part except the first (4 of 22
satellites visible). On serial links the port is reopened at 115200 and verified by
probing. If no NMEA is seen, the bridge returns to the original baud. Over TCP
the bridge's serial speed cannot be changed, so only the trim and rate commands
are sent and a warning is logged. Everything is written to receiver RAM; a
power cycle restores the stored configuration.

Hardware evidence (u-blox M8030, USB `1546:01a8`, protocol 18):

| Configuration | Bytes/s | Fixes/s | GGA:RMC (last 40–60 lines) |
|---------------|---------|---------|----------------------------|
| 1 Hz default | ≈ 600 | 1 | 1:1 |
| 0.6 `set_gps_to_10hz` (CFG-RATE only) | ≈ 964 (cap) | 9.7 | 3:21 |
| + host opens at 115200 | ≈ 991 (cap) | — | unchanged (limit is in the receiver) |
| + CFG-MSG trim | ≈ 690 | — | 4:55 (RMC+GGA don't fit 9600) |
| 0.7 plan (trim + rate + CFG-PRT 115200 + reopen) | ≈ 1923 | 10.1 | 22:22 |
| 0.7 plan at 5 Hz | — | 5.3 | all GSV parts; 22 in view, 15 used |

## 6. Security model

* Untrusted input: receiver bytes, TCP/UDP peers, gpsd/NMEA/HTTP clients. All
  of it is bounded and parsed without panics; checksums are mandatory by default.
* The HTTP API exposes position data. It binds to loopback by default, and
  `/api/*` accepts a bearer token. Put a TLS reverse proxy in front for remote access.
* MQTT: TLS with system or custom CA, mTLS, password files. Topics are
  validated (no wildcards in the base topic).
* systemd unit: dedicated `gps` user, `ProtectSystem=strict`, empty capability
  set, syscall filter, private state directory (mode 0700) for the archive
  (files 0600). Docker image: non-root, read-only root filesystem and dropped
  capabilities in the compose example.
* Writes to the receiver happen only with an explicit profile+rate or
  `init_commands`.

## 7. Verification

* `cargo test`: 94 tests, all passing. Coverage includes framing and checksum
  edge cases, every sentence handler, epoch completion paths, telemetry maths,
  config/env mapping, receiver frames (byte-compared against frames verified on
  hardware), MQTT payload builders (legacy formats, dashboard regex
  compatibility, HA, OwnTracks), HTTP routes and auth, gpsd sessions, NMEA
  TCP/UDP, sources against local sockets, the TUI rendered into a test backend,
  the dashboard's CSP and asset rules (no inline script, no remote origins, no `innerHTML`),
  and two real receiver captures (cold start; 3D fix with a genuine loss of fix).
  Capture coordinates are shifted to a public location.
* `cargo clippy --all-targets -- -D warnings`: clean. `cargo fmt`: clean.
  `cargo audit`: clean with the documented exception.
* Live, against the u-blox M8030: auto-detection (port and baud), every MQTT
  format via Mosquitto, HTTP health/REST/metrics/auth, gpsd and NMEA-TCP
  clients, `--healthcheck` exit codes, SIGINT shutdown (`offline` published,
  exit 0), 10 Hz with G86 packets at exactly 100 ms spacing, 5 Hz with the full
  sky, every dashboard mode plus the token dialog in a browser, the dark OSM
  map (tiles, overlays, zoom/follow, CSP), MGRS in API/MQTT/HA/dashboard,
  and static hold over MQTT. The Docker
  image builds and runs `--check-config`.
* Not exercised on hardware: MediaTek/Quectel profiles (command syntax from
  vendor documentation), Windows, TLS/mTLS brokers, the G86 Flutter app against
  this build.

## 8. Migrating from 0.6

| 0.6 | 0.7 |
|-----|-----|
| `prometheus_enabled/port/bind` | still accepted (aliases of `http_*`); default bind is now `127.0.0.1` |
| `set_gps_to_10hz = true` | still accepted = `receiver = "ublox"`, `update_rate_hz = 10` (now really 10 Hz) |
| `track_*` settings, lap topics | removed (ignored if present) |
| `port_name` default `/dev/ttyUSB0`, `baud_rate` 9600 | default `auto` / `0` (detect). Explicit values behave as before. |
| `GPS_TO_MQTT_*` env vars | now actually applied (C1). Check your Docker env for stale values. |
| `SAT/VEHICLES/<prn>` | `SAT/VEHICLES/<G07…>`, same payload |
| Archive written before live forwarding | written by a dedicated task, independent of outputs |
| Licence | GPL-3.0-only |

## 9. Known limitations

* Accuracy without GST/PUBX is an HDOP × 5 m estimate and is labelled as such.
* gpsd compatibility is a read-only subset (no device control, no `?DEVICE=`).
* The map needs internet access to the tile server; offline installs use the
  local plot. Bulk use of the public OSM tile servers is against their policy:
  fleets should run their own tile server or use a commercial provider.
* Quectel PAIR profiles set the rate only; message trimming and baud changes
  differ between firmware lines and are left to `init_commands`.
* The epoch model assumes UTC-timestamped position sentences. Receivers that
  emit only untimed sentences produce fixes via idle flush (1.5 s latency).
