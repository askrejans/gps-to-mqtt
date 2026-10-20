//! Outputs. Every output subscribes to the bus independently; a failing or
//! slow output never affects acquisition or the other outputs.

pub mod gpsd;
pub mod http;
pub mod mqtt;
pub mod nmea_net;
pub mod web;

use crate::config::AppConfig;
use crate::shared::Shared;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::error;

/// Human-readable list of enabled outputs (TUI header, `/api/v1/status`).
pub fn describe(config: &AppConfig) -> Vec<String> {
    let mut out = Vec::new();
    if config.mqtt_enabled {
        let formats: Vec<String> = config
            .mqtt_formats
            .iter()
            .map(|f| format!("{f:?}").to_lowercase())
            .collect();
        out.push(format!(
            "mqtt://{} [{}]",
            config.mqtt_address(),
            formats.join(",")
        ));
    }
    if config.http_enabled {
        out.push(format!("http://{}:{}", config.http_bind, config.http_port));
    }
    if let Some(a) = &config.nmea_tcp_listen {
        out.push(format!("nmea-tcp://{a}"));
    }
    if let Some(a) = &config.nmea_udp_target {
        out.push(format!("nmea-udp://{a}"));
    }
    if let Some(a) = &config.gpsd_listen {
        out.push(format!("gpsd://{a}"));
    }
    if let Some(p) = &config.telemetry_log {
        out.push(format!("archive {p}"));
    }
    out
}

pub fn spawn_all(
    config: &AppConfig,
    shared: &Arc<Shared>,
    cancel: &CancellationToken,
    set: &mut JoinSet<()>,
) {
    fn guard(name: &'static str, r: anyhow::Result<()>) {
        if let Err(e) = r {
            error!("{name} output stopped: {e:#}");
        }
    }
    if config.mqtt_enabled {
        let (c, s, k) = (config.clone(), shared.clone(), cancel.clone());
        set.spawn(async move { guard("MQTT", mqtt::run(c, s, k).await) });
    }
    if config.http_enabled {
        let (c, s, k) = (config.clone(), shared.clone(), cancel.clone());
        set.spawn(async move { guard("HTTP", http::run(&c, s, k).await) });
    }
    if let Some(addr) = config.nmea_tcp_listen.clone() {
        let (s, k, max) = (shared.clone(), cancel.clone(), config.max_clients);
        set.spawn(async move { guard("NMEA TCP", nmea_net::run_tcp(addr, max, s, k).await) });
    }
    if let Some(target) = config.nmea_udp_target.clone() {
        let (s, k) = (shared.clone(), cancel.clone());
        set.spawn(async move { guard("NMEA UDP", nmea_net::run_udp(target, s, k).await) });
    }
    if let Some(addr) = config.gpsd_listen.clone() {
        let (s, k, max) = (shared.clone(), cancel.clone(), config.max_clients);
        set.spawn(async move { guard("gpsd", gpsd::run(addr, max, s, k).await) });
    }
}
