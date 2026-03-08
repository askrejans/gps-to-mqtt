//! Prometheus metrics exposition and health endpoint.
//!
//! When `prometheus_enabled = true` a lightweight HTTP server starts on
//! `prometheus_bind:prometheus_port` and exposes two routes:
//!
//! - `GET /metrics` — Prometheus text format (scrape target)
//! - `GET /health`  — JSON health summary
//!
//! ## Metrics exposed
//!
//! | Name | Type | Description |
//! |------|------|-------------|
//! | `gps_nmea_sentences_total` | counter | Total NMEA sentences received |
//! | `gps_connected` | gauge | 1 if GPS source is connected |
//! | `gps_fix_quality` | gauge | Fix quality (0=invalid … 8=simulation) |
//! | `gps_satellites_used` | gauge | Satellites used in current fix |
//! | `gps_satellites_in_view` | gauge | Total tracked satellites |
//! | `gps_hdop` | gauge | Horizontal dilution of precision |
//! | `gps_speed_kmh` | gauge | Current speed in km/h |
//! | `gps_altitude_meters` | gauge | Altitude above sea level (m) |
//! | `gps_position_accuracy_meters` | gauge | 2D position accuracy (m, from GST) |
//! | `mqtt_connected` | gauge | 1 if MQTT broker is connected |
//! | `mqtt_messages_published_total` | counter | Total MQTT messages published |

use crate::models::{AppState, FixQuality, MqttStatus};
use anyhow::Result;
use axum::{Router, extract::State, response::IntoResponse, routing::get};
use prometheus::{Encoder, Gauge, IntCounter, IntGauge, TextEncoder};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::info;

// ---------------------------------------------------------------------------
// Global metrics — initialised once, registered with the default registry
// ---------------------------------------------------------------------------

/// Total NMEA sentences received. Incremented directly from serial/tcp tasks.
pub static NMEA_SENTENCES_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "gps_nmea_sentences_total",
        "Total NMEA sentences received from the GPS source"
    )
    .expect("register gps_nmea_sentences_total")
});

static GPS_CONNECTED: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "gps_connected",
        "1 if the GPS source is connected, 0 otherwise"
    )
    .expect("register gps_connected")
});

static GPS_FIX_QUALITY: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "gps_fix_quality",
        "GPS fix quality (0=invalid, 1=GPS, 2=DGPS, 3=PPS, 4=RTK, 5=Float RTK, 6=Estimated, 7=Manual, 8=Simulation)"
    )
    .expect("register gps_fix_quality")
});

static GPS_SATELLITES_USED: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "gps_satellites_used",
        "Satellites used in the current fix"
    )
    .expect("register gps_satellites_used")
});

static GPS_SATELLITES_IN_VIEW: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "gps_satellites_in_view",
        "Total satellites being tracked"
    )
    .expect("register gps_satellites_in_view")
});

static GPS_HDOP: LazyLock<Gauge> = LazyLock::new(|| {
    prometheus::register_gauge!("gps_hdop", "Horizontal dilution of precision")
        .expect("register gps_hdop")
});

static GPS_SPEED_KMH: LazyLock<Gauge> = LazyLock::new(|| {
    prometheus::register_gauge!("gps_speed_kmh", "Current speed in km/h")
        .expect("register gps_speed_kmh")
});

static GPS_ALTITUDE_METERS: LazyLock<Gauge> = LazyLock::new(|| {
    prometheus::register_gauge!(
        "gps_altitude_meters",
        "Altitude above sea level in metres"
    )
    .expect("register gps_altitude_meters")
});

static GPS_POSITION_ACCURACY_METERS: LazyLock<Gauge> = LazyLock::new(|| {
    prometheus::register_gauge!(
        "gps_position_accuracy_meters",
        "2D position accuracy in metres (derived from NMEA GST sentence)"
    )
    .expect("register gps_position_accuracy_meters")
});

static MQTT_CONNECTED: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "mqtt_connected",
        "1 if the MQTT broker connection is up, 0 otherwise"
    )
    .expect("register mqtt_connected")
});

/// Total MQTT messages published. Synced from AppState's AtomicU64 counter.
static MQTT_MESSAGES_PUBLISHED_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "mqtt_messages_published_total",
        "Total MQTT messages published to the broker"
    )
    .expect("register mqtt_messages_published_total")
});

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

type SharedState = Arc<RwLock<AppState>>;

/// Spawn the metrics HTTP server and the background gauge-sync task.
///
/// The server listens on `bind:port` and exposes `/metrics` and `/health`.
pub async fn spawn_metrics_server(
    bind: &str,
    port: u16,
    app_state: SharedState,
    cancel: CancellationToken,
) -> Result<()> {
    // Touch all statics to ensure they are registered before any scrape
    force_init_metrics();

    // Periodic gauge sync task
    {
        let state = Arc::clone(&app_state);
        let c = cancel.clone();
        tokio::spawn(async move {
            sync_gauges_loop(state, c).await;
        });
    }

    let addr: std::net::SocketAddr = format!("{}:{}", bind, port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid prometheus bind address '{}:{}': {}", bind, port, e))?;

    let router = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .with_state(app_state);

    info!("Prometheus metrics server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;

    Ok(())
}

/// Force-initialise all lazy metrics so they appear in /metrics even before
/// any GPS data arrives.
fn force_init_metrics() {
    let _ = &*NMEA_SENTENCES_TOTAL;
    let _ = &*GPS_CONNECTED;
    let _ = &*GPS_FIX_QUALITY;
    let _ = &*GPS_SATELLITES_USED;
    let _ = &*GPS_SATELLITES_IN_VIEW;
    let _ = &*GPS_HDOP;
    let _ = &*GPS_SPEED_KMH;
    let _ = &*GPS_ALTITUDE_METERS;
    let _ = &*GPS_POSITION_ACCURACY_METERS;
    let _ = &*MQTT_CONNECTED;
    let _ = &*MQTT_MESSAGES_PUBLISHED_TOTAL;
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let families = prometheus::gather();
    let mut buf = Vec::new();
    encoder.encode(&families, &mut buf).unwrap_or(());
    (
        [(
            axum::http::header::CONTENT_TYPE,
            encoder.format_type().to_owned(),
        )],
        buf,
    )
}

#[derive(serde::Serialize)]
struct HealthResponse {
    status: &'static str,
    gps_connected: bool,
    mqtt_connected: bool,
    mqtt_enabled: bool,
    connection_address: String,
}

async fn health_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let mqtt_connected = s.mqtt_status == MqttStatus::Connected;
    let status = if s.serial_connected { "ok" } else { "degraded" };
    axum::Json(HealthResponse {
        status,
        gps_connected: s.serial_connected,
        mqtt_connected,
        mqtt_enabled: s.mqtt_enabled,
        connection_address: s.connection_address.clone(),
    })
}

// ---------------------------------------------------------------------------
// Background gauge sync (runs every 5 s)
// ---------------------------------------------------------------------------

async fn sync_gauges_loop(state: SharedState, cancel: CancellationToken) {
    let mut last_mqtt_published: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = cancel.cancelled() => break,
        }

        let s = state.read().await;
        sync_gauges_once(&s, &mut last_mqtt_published);
    }
}

/// Sync all Prometheus gauges from a snapshot of [`AppState`].
/// Separated from the async loop so it can be called directly in tests.
fn sync_gauges_once(s: &AppState, last_mqtt_published: &mut u64) {
    GPS_CONNECTED.set(i64::from(s.serial_connected));

    if let Some(q) = s.gps_data.fix.fix_quality {
        GPS_FIX_QUALITY.set(fix_quality_to_i64(q));
    }
    if let Some(n) = s.gps_data.fix.satellites_used {
        GPS_SATELLITES_USED.set(n as i64);
    }
    if let Some(n) = s.gps_data.satellites_in_view {
        GPS_SATELLITES_IN_VIEW.set(n as i64);
    }
    if let Some(v) = s.gps_data.fix.hdop {
        GPS_HDOP.set(v);
    }
    if let Some(v) = s.gps_data.navigation.speed_kph {
        GPS_SPEED_KMH.set(v);
    }
    if let Some(v) = s.gps_data.navigation.altitude {
        GPS_ALTITUDE_METERS.set(v);
    }
    if let Some(v) = s.gps_data.navigation.position_accuracy {
        GPS_POSITION_ACCURACY_METERS.set(v);
    }

    let mqtt_up = s.mqtt_status == MqttStatus::Connected;
    MQTT_CONNECTED.set(i64::from(mqtt_up));

    // Advance the published-messages counter by the delta since last sync
    let current = s
        .messages_published
        .load(std::sync::atomic::Ordering::Relaxed);
    if current > *last_mqtt_published {
        MQTT_MESSAGES_PUBLISHED_TOTAL.inc_by(current - *last_mqtt_published);
        *last_mqtt_published = current;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fix_quality_to_i64(q: FixQuality) -> i64 {
    match q {
        FixQuality::Invalid => 0,
        FixQuality::GpsFix => 1,
        FixQuality::DgpsFix => 2,
        FixQuality::PpsFix => 3,
        FixQuality::Rtk => 4,
        FixQuality::FloatRtk => 5,
        FixQuality::Estimated => 6,
        FixQuality::Manual => 7,
        FixQuality::Simulation => 8,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MqttStatus;
    use axum::extract::State;
    use axum::response::IntoResponse;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // --- pure helpers ---

    #[test]
    fn test_fix_quality_to_i64_all_variants() {
        assert_eq!(fix_quality_to_i64(FixQuality::Invalid), 0);
        assert_eq!(fix_quality_to_i64(FixQuality::GpsFix), 1);
        assert_eq!(fix_quality_to_i64(FixQuality::DgpsFix), 2);
        assert_eq!(fix_quality_to_i64(FixQuality::PpsFix), 3);
        assert_eq!(fix_quality_to_i64(FixQuality::Rtk), 4);
        assert_eq!(fix_quality_to_i64(FixQuality::FloatRtk), 5);
        assert_eq!(fix_quality_to_i64(FixQuality::Estimated), 6);
        assert_eq!(fix_quality_to_i64(FixQuality::Manual), 7);
        assert_eq!(fix_quality_to_i64(FixQuality::Simulation), 8);
    }

    // --- gauge sync ---

    #[test]
    fn test_sync_gauges_once_mqtt_counter_delta() {
        // Only the *delta* since last sync should be added to the counter.
        // Uses serial_connected=true so this test does not zero-out GPS_CONNECTED
        // and race with other tests that check its value.
        let mut s = AppState::default();
        s.serial_connected = true;
        s.messages_published.store(20, Ordering::Relaxed);
        let mut last = 0u64;
        let before = MQTT_MESSAGES_PUBLISHED_TOTAL.get();
        sync_gauges_once(&s, &mut last);
        assert_eq!(last, 20);
        assert_eq!(MQTT_MESSAGES_PUBLISHED_TOTAL.get(), before + 20);

        // Second call with the same published count — counter must not increase again.
        let mid = MQTT_MESSAGES_PUBLISHED_TOTAL.get();
        sync_gauges_once(&s, &mut last);
        assert_eq!(MQTT_MESSAGES_PUBLISHED_TOTAL.get(), mid);
        assert_eq!(last, 20);
    }

    // --- HTTP handlers ---

    #[tokio::test]
    async fn test_health_handler_ok_when_connected() {
        let state = Arc::new(RwLock::new(AppState {
            serial_connected: true,
            mqtt_status: MqttStatus::Connected,
            mqtt_enabled: true,
            connection_address: "TCP 192.168.1.10:9001".to_string(),
            ..AppState::default()
        }));
        let resp = health_handler(State(state)).await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["gps_connected"], true);
        assert_eq!(json["mqtt_connected"], true);
        assert_eq!(json["connection_address"], "TCP 192.168.1.10:9001");
    }

    #[tokio::test]
    async fn test_health_handler_degraded_when_disconnected() {
        let state = Arc::new(RwLock::new(AppState {
            serial_connected: false,
            mqtt_status: MqttStatus::Disconnected,
            ..AppState::default()
        }));
        let resp = health_handler(State(state)).await.into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["gps_connected"], false);
        assert_eq!(json["mqtt_connected"], false);
    }

    #[tokio::test]
    async fn test_metrics_handler_content_and_metric_names() {
        force_init_metrics();
        let resp = metrics_handler().await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let ct = resp
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("text/plain"), "unexpected Content-Type: {ct}");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        for name in &[
            "gps_nmea_sentences_total",
            "gps_connected",
            "gps_fix_quality",
            "gps_satellites_used",
            "gps_hdop",
            "gps_speed_kmh",
            "mqtt_connected",
            "mqtt_messages_published_total",
        ] {
            assert!(text.contains(name), "missing metric: {name}");
        }
    }
}
