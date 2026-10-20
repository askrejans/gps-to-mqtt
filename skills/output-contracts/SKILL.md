---
name: output-contracts
description: Add or change any gps-to-mqtt output — MQTT topics/formats (legacy, json, g86, homeassistant, owntracks), HTTP/REST/SSE routes, Prometheus metrics, gpsd or NMEA servers, Fix/Sky JSON fields — without breaking G86 consumers.
---

# Output contracts

## Consumers that break silently

| Contract | Consumer | Rule |
|----------|----------|------|
| `<base>/position` (schema 1, camelCase, non-retained) | G86 Flutter app `mqtt_gps_decoder.dart` | Frozen. Changes need a new `schema` value and coordinated app work (see `../../skills/automotive-telemetry`). Static hold must never touch it. |
| Legacy topics (`/LAT`, `/SPD`, `/CRS`, `/TME`, `/DTE`, `/QTY`, `/SAT/VEHICLES/*` …) | G86 web dashboard (`g86-web-dashboard/src/pages/*.jsx`, `TabGPS.jsx` regex) | Keep names and number formats. `SAT/VEHICLES` payload must match `PRN: n, Type: X, Elevation: n, Azimuth: n, SNR: n, In View: b`. |
| `/metrics` names from 0.6 | Grafana dashboards | Add, never rename. |
| `/health` 200/503 semantics | Docker/k8s probes, `--healthcheck` | Keep. |

Check consumers before editing: `grep -rn "<topic>" ../../g86-web-dashboard/src ../../g86-racing-app*/lib`.

## Adding data

1. Add the field to `Fix`/`Sky` (`src/models.rs`) in SI units, with a unit
   suffix in the name. It then appears in `json`, the HTTP API, SSE and GeoJSON.
2. Legacy MQTT: add to `legacy_fix` with a fixed format. Test the exact string
   in `legacy_topics_keep_0_6_formats`.
3. Home Assistant: add a sensor tuple in `homeassistant_discovery` (key, name,
   template on `value_json`, unit, device class). Keep `unique_id` stable.
4. gpsd: map to the protocol field name (TPV/SKY) only if gpsd defines it.
5. Prometheus: add to `render_metrics` with HELP/TYPE. Counters end in `_total`.
6. Dashboard: see the web-dashboard skill.
7. Document in README (topic/table) and TECHNICAL.md §4.

## Adding an MQTT format

Add a `MqttFormat` variant (config list parsing is automatic), publish in
`Publisher::publish_fix`/`publish_sky`, re-send retained state in `on_connect`,
and use `try_publish` only while connected (drops are counted). Never block the
publisher. Throttling (`mqtt_max_rate_hz`) applies to state formats, never to `g86`.

## HTTP

Routes under `/api/v1/*` sit behind the token middleware automatically; `/health`
and `/metrics` stay open. Return JSON errors with correct status codes and test
with `tower::ServiceExt::oneshot` (see `output/http.rs` tests). Never expose
secrets; config goes out via `AppConfig::redacted()`.
