---
name: receiver-support
description: Add or change NMEA sentence handling, constellations, receiver profiles (u-blox/MediaTek/Quectel), input sources or epoch logic in gps-to-mqtt. Use for parsing bugs, new receivers, update-rate problems, or satellite/fix anomalies.
---

# Receiver support

## Where things live

| Concern | File |
|---------|------|
| Byte framing, checksum, talker/formatter split | `src/nmea/mod.rs` |
| Field formats (time, date, coordinates, numbers) | `src/nmea/fields.rs` |
| Sentence → epoch/aux state, satellites, validity, accuracy provenance | `src/engine.rs` |
| Constellation classification, enums, units | `src/models.rs` |
| Vendor commands, rate plans, bandwidth rule | `src/receiver.rs` |
| Port ranking, baud probing, plan application, reopen/fallback | `src/source/serial.rs` |
| TCP/UDP/gpsd/replay | `src/source/net.rs`, `src/source/replay.rs` |

## Adding a sentence

1. Add a `match s.kind` arm in `Engine::ingest`. `s.field(i)` is 0-based after
   the address and returns `""` when missing; never index raw splits.
2. Timed position sentences call `enter_epoch(time)` first and return the
   pending fix. Untimed ones write an `aux` slot as `Fresh { value, at: now }`
   so they expire after `AUX_MAX_AGE`.
3. Parse with `num`/`int`/`parse_*`, which reject non-finite values. Validate ranges.
4. Test with `with_checksum("GPXXX,…")` bodies. Cover empty fields, both NMEA
   variants (e.g. VTG 2.3+ and legacy), and the mode character.
5. Expose new data through `Fix`/`Sky` (SI units), then follow the
   output-contracts skill.

## Receiver profiles

- `receiver::plan(profile, rate, custom, link_baud)` returns the commands plus
  an optional reopen baud. Order: custom commands, sentence trim, rate, and the
  baud change **last** (a real UART switches immediately).
- Bandwidth rule (`exceeds_link`): RMC+GGA ≈ 150 B/epoch × rate + ≈ 700 B/s
  GSA/GSV burst against 80 % of baud/10. Measured on a u-blox M8030: USB output
  is capped by the UART1 baud. Counting only RMC+GGA lost the GSV parts at 5 Hz.
- Build binary frames with `ubx_frame` (Fletcher-8) or NMEA with `with_checksum`.
  Byte-compare new frames in tests, ideally against frames verified on hardware.
- Only RAM settings. Never send save commands (UBX-CFG-CFG save, PMTK flash, PAIR save).
- Verify on hardware (hardware-validation skill). Record measured bytes/s,
  fixes/s and the GGA:RMC ratio in TECHNICAL.md §5.

## Constellations

`Constellation::classify(talker, prn)` covers NMEA ≤ 4.0 extended numbering
(GLONASS 65–96 under `GP`/`GN`) and 4.10+ per-system numbering. Satellites are
keyed by (constellation, PRN). GSA used-sets come from field 18 (system ID) or
the talker. `GN` without an ID matches by PRN.

## Pitfalls

- chrono omits `.0` fractions; never compare formatted times.
- RMC `V` or GGA quality 0 must give `valid = false` and `mode = NoFix`.
- The G86 encoder (`position.rs`) has its own stricter checks. Do not route it
  through the engine and do not relax it.
- Idle flush judges aux freshness against the epoch's last input, not flush time.
