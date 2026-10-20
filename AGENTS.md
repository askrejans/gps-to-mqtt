# gps-to-mqtt — agent guide

Universal NMEA-0183 GNSS gateway (Rust, tokio). Part of `g86racing`: read the
[project guidance](../../AGENTS.md) before changing anything another component
consumes. Preserve the current branch and any local edits.

Design, output contracts, receiver measurements and the security model:
[TECHNICAL.md](TECHNICAL.md). User documentation: [README.md](README.md).

## Skills

Read the matching skill before starting:

| Task | Skill |
|------|-------|
| New sentence, constellation, receiver profile, source, or anything in parsing/epochs | [skills/receiver-support](skills/receiver-support/SKILL.md) |
| New or changed output: MQTT format/topic, HTTP route, gpsd/NMEA servers, JSON fields | [skills/output-contracts](skills/output-contracts/SKILL.md) |
| Web dashboard (`web/`) | [skills/web-dashboard](skills/web-dashboard/SKILL.md) |
| Testing with a real receiver, recording fixtures | [skills/hardware-validation](skills/hardware-validation/SKILL.md) |
| Version bump, packages, Docker, dependencies, audit | [skills/release](skills/release/SKILL.md) |
| Cross-component telemetry contracts (G86 app, dashboard, API) | [../../skills/automotive-telemetry](../../skills/automotive-telemetry/SKILL.md) |

## Map

```
src/nmea/       framing (LineFramer), Sentence + checksum, field parsers   ← untrusted input
src/engine.rs   epoch aggregation → Fix, satellites, accuracy provenance
src/hold.rs     static hold    src/mgrs.rs  UTM/MGRS    src/telemetry.rs  dynamics
src/position.rs G86 /position packet (contract — independent strict checks)
src/receiver.rs UBX / PMTK / PAIR builders, rate plans
src/source/     serial (auto-detect, plan apply), tcp, udp, gpsd, replay
src/hub.rs      single consumer of source bytes → bus + snapshot + archive
src/shared.rs   broadcast bus, snapshot, counters, health
src/output/     mqtt, http (+ web assets), nmea_net, gpsd
src/ui/         ratatui terminal dashboard
web/            browser dashboard, embedded with include_str!
tests/fixtures/ real receiver captures (anonymised)
scripts/        build_packages.sh, anonymize_nmea.py, utm_reference.py
```

## Commands

```bash
cargo test                                    # must pass; includes real-capture fixtures
cargo clippy --all-targets -- -D warnings     # must be clean
cargo fmt
cargo audit                                   # exceptions only in .cargo/audit.toml, with a reason
cargo run -- --replay tests/fixtures/ublox-m8-3d-fix-1hz.nmea   # no hardware needed
cargo run -- --list-ports --probe
```

## Invariants

1. **Acquisition never waits on an output.** Outputs subscribe to the lossy
   bus; drops are counted. Only the archive has its own bounded queue.
2. **One parser path.** Every byte goes through `LineFramer` → `Sentence::parse`
   in the hub. Sources forward raw bytes; they never parse.
3. **Untrusted input is bounded and never panics.** Receiver bytes, network
   peers, HTTP/gpsd clients. No `unwrap` on input, no unchecked slicing.
4. **SI units at rest.** m, m/s, degrees. Convert only at output boundaries,
   with explicit field names (`speed_mps`, `altitude_m`).
5. **Contracts are frozen unless versioned.** The G86 `/position` packet
   (schema 1) and the legacy topic names/formats used by the G86 dashboard.
   See the output-contracts skill.
6. **The device is written only when configured.** `receiver = "generic"` with
   no `init_commands` sends nothing. Plans write receiver RAM only, never flash.
7. **No real positions in the repo.** Anonymise captures with
   `scripts/anonymize_nmea.py`; examples use the public reference point
   56.9637 N, 24.2289 E.
8. **The web dashboard has no remote resources** except map tile images from
   `http_map_tiles`, and no `innerHTML`. A test enforces this.
9. **Config is flat** (`mqtt_host`, not `[mqtt] host`) so every key maps to
   `GPS_TO_MQTT_<KEY>`. New keys need a serde default, validation,
   `example.settings.toml`, the README table, and a test.

## Conventions

- Edition 2024, `anyhow` for application errors, `thiserror` for typed ones,
  `tracing` for logs. Rate-limit anything that can fire at sensor frequency.
- Tests sit next to the code (`#[cfg(test)] mod tests`). Prefer real fixtures
  and local sockets over mocks.
- Comments explain *why*: receiver quirks, standards, measured behaviour.
- Licence: GPL-3.0-only.
