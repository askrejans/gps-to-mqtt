---
name: hardware-validation
description: Test gps-to-mqtt against a real GNSS receiver — detection, rates, outputs, dashboards — record fixtures safely, and leave the receiver as found.
---

# Hardware validation

## Before

- Ask before writing to a device that is not obviously a bench unit.
  Profiles write RAM only, but say so.
- `gps-to-mqtt --list-ports --probe` shows the port, baud and identification text.
- Use a throwaway broker: `mosquitto -c conf` with `listener 18830 127.0.0.1`
  and `allow_anonymous true` in a temp directory.

## Checks

| What | How | Pass |
|------|-----|------|
| Detection | `--headless` with defaults | log shows `Detected NMEA receiver on … @ … baud` |
| Rate | counters from `/metrics` over ≥ 10 s (`gps_fixes_total`, `gps_bytes_received_total`) | fixes/s ≈ rate; RMC:GGA ≈ 1:1 in `/api/v1/nmea?limit=60` |
| Full sky at rate | `/api/v1/sky` `in_view` vs used | in_view ≥ used |
| MQTT | `mosquitto_sub -p 18830 -t '#' -v -W 5` | every enabled format present |
| G86 packets | sub `<base>/position`; check `sequence` and `timestampMs` deltas | contiguous, monotonic, 1000/rate ms |
| Health | `--healthcheck` exit codes, `/health` | 0 with fix, 1 without |
| Shutdown | SIGINT | `<base>/status offline`, exit 0 |

The 1-second gauges (`gps_fix_rate_hz`) jitter, so measure with counter deltas.
macOS has no `timeout`; use `mosquitto_sub -W` and `sh -c '… & pid=$!; …; kill -INT $pid'`.

## Recording fixtures

1. Capture raw bytes (or via the NMEA TCP output) for ≥ 10 s.
2. `python3 -I scripts/anonymize_nmea.py raw.nmea tests/fixtures/<receiver>-<situation>.nmea`.
   **Never commit real coordinates.** Check with `grep` for the original degrees.
3. Add a test in `engine.rs` `fixture_tests` asserting what the capture shows
   (fix count, validity, satellites, G86 packets).

## Leave it as found

u-blox: reload the stored configuration into RAM with UBX-CFG-CFG
(clear 0, save 0, load 0xFFFF), or ask the user to power-cycle. Then confirm
with `--list-ports --probe` that it is back at its original baud. Stop any
brokers and bridge processes you started.
