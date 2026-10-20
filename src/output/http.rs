//! HTTP API: health probe, Prometheus metrics, REST snapshot and live SSE.
//!
//! | Route                  | Auth  | Description                                  |
//! |------------------------|-------|----------------------------------------------|
//! | `GET /health`          | open  | 200 when source up + fresh valid fix, else 503 |
//! | `GET /metrics`         | open  | Prometheus text exposition                   |
//! | `GET /api/v1/status`   | token | wiring, health and counters                  |
//! | `GET /api/v1/fix`      | token | latest fix (404 before the first fix)        |
//! | `GET /api/v1/sky`      | token | satellites in view                           |
//! | `GET /api/v1/geojson`  | token | latest position as a GeoJSON Feature         |
//! | `GET /api/v1/nmea`     | token | recent raw sentences (`?limit=`)             |
//! | `GET /api/v1/stream`   | token | Server-Sent Events: `fix`, `sky`, `position` |
//!
//! "token" routes require `Authorization: Bearer <http_token>` when a token is
//! configured.

use crate::config::AppConfig;
use crate::shared::{BusEvent, Counters, LinkStatus, Shared};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response, sse},
    routing::get,
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Fix older than this makes `/health` report `no_fix`.
const HEALTH_MAX_FIX_AGE_S: f64 = 5.0;

#[derive(Clone)]
struct Ctx {
    shared: Arc<Shared>,
    token: Option<Arc<str>>,
    config: Arc<serde_json::Value>,
}

/// Router options derived from [`AppConfig`].
#[derive(Default)]
pub struct Options {
    pub token: Option<String>,
    pub cors_origin: Option<String>,
    pub ui: bool,
    /// Tile origin allowed by the dashboard CSP (`None` = map disabled).
    pub map_origin: Option<String>,
    /// Redacted configuration served at `/api/v1/config`.
    pub config: serde_json::Value,
}

impl Options {
    pub fn from_config(c: &AppConfig) -> Self {
        Self {
            token: c.http_token.clone(),
            cors_origin: c.http_cors_origin.clone(),
            ui: c.http_ui,
            map_origin: (!c.http_map_tiles.is_empty())
                .then(|| crate::config::map_origin(&c.http_map_tiles).ok())
                .flatten(),
            config: c.redacted(),
        }
    }
}

pub fn router(shared: Arc<Shared>, opts: Options) -> Router {
    let Options {
        token,
        cors_origin,
        ui,
        map_origin,
        config,
    } = opts;
    let ctx = Ctx {
        shared,
        token: token.filter(|t| !t.is_empty()).map(Arc::from),
        config: Arc::new(config),
    };
    let api = Router::new()
        .route("/api/v1/config", get(config_handler))
        .route("/api/v1/status", get(status))
        .route("/api/v1/fix", get(fix))
        .route("/api/v1/sky", get(sky))
        .route("/api/v1/geojson", get(geojson))
        .route("/api/v1/nmea", get(nmea))
        .route("/api/v1/stream", get(stream_events))
        .route_layer(middleware::from_fn_with_state(ctx.clone(), require_token));
    let cors = cors_origin.and_then(|o| HeaderValue::from_str(&o).ok());
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .merge(api);
    if ui {
        app = app.merge(super::web::router(map_origin.as_deref()));
    }
    app.layer(middleware::from_fn(move |req: Request, next: Next| {
        let cors = cors.clone();
        async move {
            let mut res = next.run(req).await;
            let h = res.headers_mut();
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            if let Some(origin) = cors {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    HeaderValue::from_static("authorization"),
                );
            }
            res
        }
    }))
    .with_state(ctx)
}

pub async fn run(config: &AppConfig, shared: Arc<Shared>, cancel: CancellationToken) -> Result<()> {
    let addr: std::net::SocketAddr = format!("{}:{}", config.http_bind, config.http_port)
        .parse()
        .context("invalid http_bind/http_port")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding HTTP server to {addr}"))?;
    if config.http_token.is_none() && !addr.ip().is_loopback() {
        tracing::warn!(
            "HTTP API on {addr} has no http_token; /api is readable by anyone on the network"
        );
    }
    info!("HTTP API listening on http://{addr}");
    if config.http_ui {
        info!("Web dashboard on http://{addr}/");
    }
    let app = router(shared, Options::from_config(config));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;
    Ok(())
}

/// Constant-time comparison so token checks don't leak length-prefix timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_token(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    if let Some(expected) = &ctx.token {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !ct_eq(presented.as_bytes(), expected.as_bytes()) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({"error": "unauthorized"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn health(State(ctx): State<Ctx>) -> Response {
    let h = ctx.shared.health(HEALTH_MAX_FIX_AGE_S);
    let code = if h.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(h)).into_response()
}

async fn status(State(ctx): State<Ctx>) -> Response {
    let s = &ctx.shared;
    let c = &s.counters;
    let snap = s.read();
    Json(json!({
        "info": s.info,
        "health": {
            "source": snap.source,
            "source_detail": snap.source_detail,
            "mqtt": snap.mqtt,
            "fix_age_s": snap.fix_age_s(Instant::now()),
            "sentence_rate_hz": snap.sentence_rate,
            "fix_rate_hz": snap.fix_rate,
        },
        "counters": {
            "bytes": Counters::get(&c.bytes),
            "sentences": Counters::get(&c.sentences),
            "checksum_errors": Counters::get(&c.checksum_errors),
            "malformed": Counters::get(&c.malformed),
            "fixes": Counters::get(&c.fixes),
            "mqtt_published": Counters::get(&c.mqtt_published),
            "mqtt_dropped": Counters::get(&c.mqtt_dropped),
            "archive_errors": Counters::get(&c.archive_errors),
            "clients": Counters::get(&c.clients),
        },
        "messages": snap.messages,
    }))
    .into_response()
}

async fn config_handler(State(ctx): State<Ctx>) -> Response {
    Json(ctx.config.as_ref().clone()).into_response()
}

async fn fix(State(ctx): State<Ctx>) -> Response {
    match ctx.shared.read().fix.clone() {
        Some(f) => Json(f).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "no fix yet"}))).into_response(),
    }
}

async fn sky(State(ctx): State<Ctx>) -> Response {
    Json(ctx.shared.read().sky.clone()).into_response()
}

async fn geojson(State(ctx): State<Ctx>) -> Response {
    let fix = ctx.shared.read().fix.clone();
    let Some(fix) = fix else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "no fix yet"}))).into_response();
    };
    let geometry = match (fix.position(), fix.altitude_m) {
        (Some((lat, lon)), Some(alt)) => json!({"type": "Point", "coordinates": [lon, lat, alt]}),
        (Some((lat, lon)), None) => json!({"type": "Point", "coordinates": [lon, lat]}),
        (None, _) => serde_json::Value::Null,
    };
    (
        [(header::CONTENT_TYPE, "application/geo+json")],
        json!({"type": "Feature", "geometry": geometry, "properties": &*fix}).to_string(),
    )
        .into_response()
}

#[derive(Deserialize)]
struct Limit {
    limit: Option<usize>,
}

async fn nmea(State(ctx): State<Ctx>, Query(q): Query<Limit>) -> Response {
    let snap = ctx.shared.read();
    let limit = q.limit.unwrap_or(50).min(snap.raw.len());
    let mut body = String::new();
    for line in snap.raw.iter().skip(snap.raw.len() - limit) {
        body.push_str(line);
        body.push_str("\r\n");
    }
    (
        [(header::CONTENT_TYPE, "text/plain; charset=us-ascii")],
        body,
    )
        .into_response()
}

async fn stream_events(
    State(ctx): State<Ctx>,
) -> sse::Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let rx = ctx.shared.bus.subscribe();
    let events = stream::unfold(rx, |mut rx| async move {
        loop {
            let event = match rx.recv().await {
                Ok(BusEvent::Fix(f)) => sse::Event::default().event("fix").json_data(&*f).ok(),
                Ok(BusEvent::Sky(s)) => sse::Event::default().event("sky").json_data(&*s).ok(),
                Ok(BusEvent::G86(p)) => Some(sse::Event::default().event("position").data(&*p)),
                Ok(BusEvent::Nmea(_)) => None,
                Err(broadcast::error::RecvError::Lagged(_)) => None,
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            if let Some(e) = event {
                return Some((Ok(e), rx));
            }
        }
    });
    sse::Sse::new(events).keep_alive(sse::KeepAlive::new().interval(Duration::from_secs(15)))
}

// ── Prometheus ───────────────────────────────────────────────────────────

fn metric(out: &mut String, name: &str, kind: &str, help: &str, value: impl std::fmt::Display) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
    );
}

pub fn render_metrics(shared: &Shared) -> String {
    let c = &shared.counters;
    let snap = shared.read();
    let fix = snap.fix.as_deref();
    let nan = f64::NAN;
    let mut o = String::with_capacity(4096);
    let _ = writeln!(
        o,
        "# HELP gps_to_mqtt_build_info Build information\n# TYPE gps_to_mqtt_build_info gauge\ngps_to_mqtt_build_info{{version=\"{}\"}} 1",
        shared.info.version
    );
    metric(
        &mut o,
        "gps_bytes_received_total",
        "counter",
        "Bytes received from the GNSS source",
        Counters::get(&c.bytes),
    );
    metric(
        &mut o,
        "gps_nmea_sentences_total",
        "counter",
        "Checksum-valid NMEA sentences received",
        Counters::get(&c.sentences),
    );
    metric(
        &mut o,
        "gps_checksum_errors_total",
        "counter",
        "Sentences rejected for a bad checksum",
        Counters::get(&c.checksum_errors),
    );
    metric(
        &mut o,
        "gps_malformed_sentences_total",
        "counter",
        "Lines rejected as malformed",
        Counters::get(&c.malformed),
    );
    metric(
        &mut o,
        "gps_fixes_total",
        "counter",
        "Navigation epochs emitted",
        Counters::get(&c.fixes),
    );
    metric(
        &mut o,
        "gps_connected",
        "gauge",
        "1 if the GNSS source is connected and sending",
        i32::from(snap.source == LinkStatus::Connected),
    );
    metric(
        &mut o,
        "gps_fix_valid",
        "gauge",
        "1 if the latest fix is valid",
        i32::from(fix.is_some_and(|f| f.valid)),
    );
    metric(
        &mut o,
        "gps_fix_age_seconds",
        "gauge",
        "Seconds since the latest fix",
        snap.fix_age_s(Instant::now()).unwrap_or(nan),
    );
    metric(
        &mut o,
        "gps_fix_quality",
        "gauge",
        "GGA fix quality (0 invalid … 8 simulation)",
        fix.map_or(0, |f| f.quality.code()),
    );
    metric(
        &mut o,
        "gps_satellites_used",
        "gauge",
        "Satellites used in the fix",
        fix.and_then(|f| f.satellites_used).unwrap_or(0),
    );
    metric(
        &mut o,
        "gps_satellites_in_view",
        "gauge",
        "Satellites in view",
        snap.sky.in_view,
    );
    metric(
        &mut o,
        "gps_hdop",
        "gauge",
        "Horizontal dilution of precision",
        fix.and_then(|f| f.hdop).unwrap_or(nan),
    );
    metric(
        &mut o,
        "gps_speed_kmh",
        "gauge",
        "Speed over ground, km/h",
        fix.and_then(|f| f.speed_kph()).unwrap_or(nan),
    );
    metric(
        &mut o,
        "gps_altitude_meters",
        "gauge",
        "Altitude above MSL, m",
        fix.and_then(|f| f.altitude_m).unwrap_or(nan),
    );
    metric(
        &mut o,
        "gps_position_accuracy_meters",
        "gauge",
        "Horizontal accuracy, m (see accuracy_source)",
        fix.and_then(|f| f.accuracy_m).unwrap_or(nan),
    );
    metric(
        &mut o,
        "gps_sentence_rate_hz",
        "gauge",
        "NMEA sentences per second",
        snap.sentence_rate,
    );
    metric(
        &mut o,
        "gps_fix_rate_hz",
        "gauge",
        "Fixes per second",
        snap.fix_rate,
    );
    metric(
        &mut o,
        "mqtt_connected",
        "gauge",
        "1 if the MQTT broker connection is up",
        i32::from(snap.mqtt == LinkStatus::Connected),
    );
    metric(
        &mut o,
        "mqtt_messages_published_total",
        "counter",
        "MQTT messages handed to the client",
        Counters::get(&c.mqtt_published),
    );
    metric(
        &mut o,
        "mqtt_messages_dropped_total",
        "counter",
        "Live MQTT messages dropped (disconnected or queue full)",
        Counters::get(&c.mqtt_dropped),
    );
    metric(
        &mut o,
        "archive_errors_total",
        "counter",
        "Archive packets that could not be written",
        Counters::get(&c.archive_errors),
    );
    metric(
        &mut o,
        "output_clients",
        "gauge",
        "Connected NMEA/gpsd TCP clients",
        Counters::get(&c.clients),
    );
    o
}

async fn metrics(State(ctx): State<Ctx>) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        render_metrics(&ctx.shared),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Fix;
    use crate::shared::Info;
    use axum::body::Body;
    use tower::ServiceExt;

    fn opts(token: Option<&str>, cors: Option<&str>) -> Options {
        Options {
            token: token.map(String::from),
            cors_origin: cors.map(String::from),
            ui: true,
            map_origin: None,
            config: AppConfig::default().redacted(),
        }
    }

    fn shared_with_fix() -> Arc<Shared> {
        let shared = Shared::new(
            Info {
                version: "test",
                ..Info::default()
            },
            true,
        );
        shared.set_source(LinkStatus::Connected, "/dev/test");
        {
            let mut s = shared.write();
            s.fix = Some(Arc::new(Fix {
                valid: true,
                latitude: Some(56.9),
                longitude: Some(24.1),
                altitude_m: Some(10.0),
                ..Fix::default()
            }));
            s.last_fix_at = Some(Instant::now());
            s.push_raw(Arc::from("$GPGGA,1*00"));
        }
        shared
    }

    async fn get(app: &Router, path: &str, token: Option<&str>) -> (StatusCode, String, HeaderMap) {
        let mut req = Request::builder().uri(path);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap(), headers)
    }

    #[tokio::test]
    async fn health_and_metrics_are_open() {
        let app = router(shared_with_fix(), opts(Some("secret"), None));
        let (code, body, headers) = get(&app, "/health", None).await;
        assert_eq!(code, StatusCode::OK);
        assert!(body.contains("\"status\":\"ok\""));
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let (code, body, _) = get(&app, "/metrics", None).await;
        assert_eq!(code, StatusCode::OK);
        for name in [
            "gps_nmea_sentences_total",
            "gps_connected 1",
            "gps_fix_valid 1",
            "mqtt_connected 0",
            "gps_to_mqtt_build_info{version=\"test\"} 1",
        ] {
            assert!(body.contains(name), "missing {name}");
        }
    }

    #[tokio::test]
    async fn health_is_503_without_fix() {
        let shared = Shared::new(Info::default(), false);
        let app = router(shared, opts(None, None));
        assert_eq!(
            get(&app, "/health", None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            get(&app, "/api/v1/fix", None).await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn api_requires_token_when_configured() {
        let app = router(
            shared_with_fix(),
            opts(Some("secret"), Some("https://dash.local")),
        );
        assert_eq!(
            get(&app, "/api/v1/fix", None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get(&app, "/api/v1/fix", Some("wrong")).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (code, body, headers) = get(&app, "/api/v1/fix", Some("secret")).await;
        assert_eq!(code, StatusCode::OK);
        assert!(body.contains("\"latitude\":56.9"));
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://dash.local"
        );
    }

    #[tokio::test]
    async fn geojson_nmea_and_status() {
        let app = router(shared_with_fix(), opts(None, None));
        let (_, body, headers) = get(&app, "/api/v1/geojson", None).await;
        assert_eq!(headers[header::CONTENT_TYPE], "application/geo+json");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["geometry"]["coordinates"], json!([24.1, 56.9, 10.0]));
        assert_eq!(
            get(&app, "/api/v1/nmea?limit=5", None).await.1,
            "$GPGGA,1*00\r\n"
        );
        let (_, body, _) = get(&app, "/api/v1/status", None).await;
        assert!(body.contains("\"source_detail\":\"/dev/test\""));
        let (_, body, _) = get(&app, "/api/v1/config", None).await;
        assert!(body.contains("\"mqtt_host\":\"localhost\""));
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
