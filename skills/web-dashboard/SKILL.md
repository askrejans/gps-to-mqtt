---
name: web-dashboard
description: Change the gps-to-mqtt browser dashboard in web/ (modes, panels, canvases, map, styling) while keeping its CSP, security and no-build constraints.
---

# Web dashboard

`web/index.html`, `web/app.css`, `web/app.js` and `web/icon.svg` are embedded
with `include_str!` (`src/output/web.rs`). Edit, then `cargo build`. There is
no bundler, framework or npm; keep it that way.

## Hard rules (enforced by tests in `output/web.rs`)

- No inline `<script>`, no `innerHTML`, no `http(s)://` in the assets. Build
  DOM with `el()`/`kv()` and set text with `textContent`, because receiver
  messages and NMEA are untrusted.
- Only `'self'` in the CSP, plus the configured tile origin for images. New
  external resources are not allowed. Map tiles come from config.
- Data only via the public API (`api/v1/*`, relative paths for reverse proxies).
  Send `Authorization` via `headers()`. The SSE stream is read with `fetch`.
  The UI is read-only; never add endpoints that change config or the receiver.

## Structure of app.js

State `S` → `schedule()` (one rAF) → `render()` draws the status bar, the
readout and **only the visible mode**. Data arrives via `onFix`/`onSky` (SSE),
`pollStatus` (2 s), `pollNmea` (1 s, System mode only) and `loadOnce` (config).
Canvases go through `canvas(id)` (DPR-aware, returns `{ctx,w,h}` or null when hidden).

Add a mode: a key in the nav, a `<section class="mode" id="mode-x" hidden>`,
an entry in `MODES` and the `render()` table, and a render function.

## Style

Professional instrument panel: function over form, no decorative text or
effects. Live values use `--display` with `--glow` in the numeric font, labels
are uppercase micro-labels, status comes from LEDs/lamps (`ok` / `warn` /
`bad`) and segment meters (`meter()`, `bipolar()`). Fixed-width numbers use
`tabular-nums`; reserve width for values that change length (see the UTC clock).

## Verify

Run with replay (no hardware, anonymised position):
`GPS_TO_MQTT_HTTP_ENABLED=true GPS_TO_MQTT_REPLAY_LOOP=true cargo run -- --headless --replay tests/fixtures/ublox-m8-3d-fix-1hz.nmea`,
open `http://127.0.0.1:9090/`, check every mode at desktop and phone widths,
with and without `GPS_TO_MQTT_HTTP_TOKEN`, and with `GPS_TO_MQTT_HTTP_MAP_TILES=""`.
Assets are compiled in, so restart after rebuilding and hard-reload; changing
only the hash does not reload.
