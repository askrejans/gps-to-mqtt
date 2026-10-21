//! MQTT output.
//!
//! Formats (any combination, see [`MqttFormat`]):
//!
//! | Format          | Topic(s)                                   | Retained |
//! |-----------------|--------------------------------------------|----------|
//! | `legacy`        | `<base>/LAT`, `<base>/SPD`, … one value each | yes (`mqtt_retain`) |
//! | `json`          | `<base>/fix`, `<base>/sky`                  | yes (`mqtt_retain`) |
//! | `g86`           | `<base>/position` (G86 external GPS packet) | never    |
//! | `homeassistant` | `<prefix>/…/config` discovery + JSON topics | yes      |
//! | `owntracks`     | `owntracks/<user>/<device>`                 | yes      |
//!
//! Availability: `<base>/status` is `online`/`offline` (retained; `offline` is the
//! broker-published last will).
//!
//! **On demand** (for consumers on slow links, such as the G86 display): a
//! message on `<base>/position/get` whose payload is a reply topic is answered
//! with the latest valid fix on that topic: `latitude`, `longitude`,
//! `speedMps`, `headingDeg`, `lateralG`, `longitudinalG`, `timestampMs`.
//! The consumer asks again when it has used the answer, so it never builds a
//! backlog. No answer without a valid fix from the last 2 s.
//!
//! Publishing never blocks the pipeline: while the broker is unreachable live
//! values are dropped and counted, and every retained state topic is re-sent
//! after reconnecting.

use crate::config::{AppConfig, MqttFormat};
use crate::models::{
    AccuracySource, Constellation, Fix, M_TO_FT, MPS_TO_KNOTS, MPS_TO_KPH, MPS_TO_MPH, Sky,
};
use crate::shared::{BusEvent, Counters, LinkStatus, Shared};
use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, Transport};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const RECONNECT_MAX: Duration = Duration::from_secs(60);

pub fn client_id(config: &AppConfig) -> String {
    config.mqtt_client_id.clone().unwrap_or_else(|| {
        // Unique per process: two bridges with the same ID would kick each other off.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        format!("gps-to-mqtt-{:x}{:04x}", std::process::id(), nanos & 0xffff)
    })
}

fn options(config: &AppConfig, client_id: &str) -> Result<MqttOptions> {
    let mut o = MqttOptions::new(client_id, &config.mqtt_host, config.mqtt_port);
    o.set_keep_alive(Duration::from_secs(config.mqtt_keep_alive_s.max(5)));
    o.set_clean_session(true);
    o.set_request_channel_capacity(1024);
    o.set_max_packet_size(256 * 1024, 256 * 1024);
    o.set_last_will(LastWill::new(
        format!("{}/status", config.base_topic()),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    if let Some(user) = &config.mqtt_username {
        o.set_credentials(user, config.mqtt_password()?.unwrap_or_default());
    }
    if config.mqtt_use_tls || config.mqtt_ca_file.is_some() {
        let transport = match &config.mqtt_ca_file {
            Some(ca) => {
                let ca = std::fs::read(ca).with_context(|| format!("reading mqtt_ca_file {ca}"))?;
                let client_auth =
                    match (&config.mqtt_client_cert_file, &config.mqtt_client_key_file) {
                        (Some(c), Some(k)) => Some((
                            std::fs::read(c).with_context(|| format!("reading {c}"))?,
                            std::fs::read(k).with_context(|| format!("reading {k}"))?,
                        )),
                        _ => None,
                    };
                Transport::tls(ca, client_auth, None)
            }
            None => Transport::tls_with_default_config(),
        };
        o.set_transport(transport);
    }
    Ok(o)
}

/// One outgoing message.
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

// ── Payload builders (pure, unit-tested) ─────────────────────────────────

/// Legacy scalar topics (suffixes relative to the base topic).
pub fn legacy_fix(fix: &Fix) -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = Vec::with_capacity(32);
    if let Some((lat, lon)) = fix.position() {
        v.push(("/LAT", lat.to_string()));
        v.push(("/LNG", lon.to_string()));
        if let Some(m) = &fix.mgrs {
            v.push(("/MGRS", m.clone()));
        }
        if let Some(m) = &fix.mgrs_8 {
            v.push(("/MGRS_8", m.clone()));
        }
        if let Some(alt) = fix.altitude_m {
            v.push(("/ALT", format!("{alt:.1}")));
            v.push(("/ALT_FT", format!("{:.1}", alt * M_TO_FT)));
        }
    }
    if let Some(mps) = fix.speed_mps {
        let kph = format!("{:.1}", mps * MPS_TO_KPH);
        v.push(("/SPD", kph.clone()));
        v.push(("/SPD_KPH", kph));
        v.push(("/SPD_MPH", format!("{:.1}", mps * MPS_TO_MPH)));
        v.push(("/SPD_KTS", format!("{:.2}", mps * MPS_TO_KNOTS)));
        // Course is GPS noise when stationary.
        if mps * MPS_TO_KPH >= 3.0
            && let Some(c) = fix.course_deg
        {
            v.push(("/CRS", c.to_string()));
        }
    }
    if let Some(n) = fix.satellites_used {
        v.push(("/SATS", n.to_string()));
    }
    if let Some(n) = fix.satellites_in_view {
        v.push(("/SAT/GLOBAL/NUM", n.to_string()));
    }
    if let Some(h) = fix.hdop {
        v.push(("/HDOP", h.to_string()));
    }
    if let Some(x) = fix.vdop {
        v.push(("/VDOP", format!("{x:.2}")));
    }
    if let Some(x) = fix.pdop {
        v.push(("/PDOP", format!("{x:.2}")));
    }
    v.push(("/QTY", fix.quality.code().to_string()));
    if let Some(t) = fix.utc_time {
        v.push(("/TME", t.to_string()));
    }
    if let Some(d) = fix.utc_date {
        v.push(("/DTE", d.to_string()));
    }
    if fix.accuracy_source == Some(AccuracySource::Gst)
        && let Some(a) = fix.accuracy_m
    {
        v.push(("/POSITION_ACCURACY", format!("{a:.2}")));
    }
    if let Some(h) = fix.heading_deg {
        v.push(("/TRUE_HEADING", format!("{h:.1}")));
    }
    if let Some(r) = fix.rate_of_turn_dps {
        v.push(("/HEADING_RATE_GPS", format!("{r:.2}")));
    }
    if let Some(t) = &fix.telemetry {
        let f = |x: Option<f64>, p: usize| format!("{:.*}", p, x.unwrap_or(0.0));
        v.push(("/ACCEL_LONG_MPS2", f(t.longitudinal_accel_mps2, 3)));
        v.push(("/ACCEL_LONG_G", f(t.longitudinal_g, 3)));
        v.push(("/ACCEL_LAT_MPS2", f(t.lateral_accel_mps2, 3)));
        v.push(("/ACCEL_LAT_G", f(t.lateral_g, 3)));
        v.push(("/COMBINED_G", f(t.combined_g, 3)));
        v.push(("/HEADING_RATE", f(t.yaw_rate_dps, 2)));
        v.push(("/DISTANCE", format!("{:.1}", t.distance_m)));
        if t.max_speed_mps > 0.0 {
            v.push(("/MAX_SPEED", format!("{:.1}", t.max_speed_mps * MPS_TO_KPH)));
            v.push((
                "/MAX_SPEED_MPH",
                format!("{:.1}", t.max_speed_mps * MPS_TO_MPH),
            ));
        }
        v.push(("/BRAKING", if t.braking { "1" } else { "0" }.into()));
    }
    v
}

fn legacy_type(c: Constellation) -> &'static str {
    // Names match the 0.6 payload parsed by the G86 dashboard.
    match c {
        Constellation::Gps => "Gps",
        Constellation::Sbas => "Sbas",
        Constellation::Glonass => "Glonass",
        Constellation::Galileo => "Galileo",
        Constellation::Beidou => "Beidou",
        Constellation::Qzss => "Qzss",
        Constellation::Navic => "Navic",
        Constellation::Unknown => "Unknown",
    }
}

/// `/SAT/VEHICLES/<id>` → 0.6-compatible text payload.
pub fn legacy_sky(sky: &Sky) -> Vec<(String, String)> {
    let na = |v: Option<i32>| v.map(|x| x.to_string()).unwrap_or_else(|| "N/A".into());
    sky.satellites
        .iter()
        .map(|s| {
            (
                format!("/SAT/VEHICLES/{}", s.id()),
                format!(
                    "PRN: {}, Type: {}, Elevation: {}, Azimuth: {}, SNR: {}, In View: {}",
                    s.prn,
                    legacy_type(s.constellation),
                    na(s.elevation_deg),
                    na(s.azimuth_deg),
                    na(s.snr_dbhz),
                    s.snr_dbhz.unwrap_or(0) > 0
                ),
            )
        })
        .collect()
}

/// OwnTracks `_type: location`.
pub fn owntracks(fix: &Fix, tid: &str) -> Option<String> {
    let (lat, lon) = fix.position()?;
    let tst = fix
        .time
        .map(|t| t.timestamp())
        .unwrap_or(fix.received_ms / 1000);
    let mut doc = json!({
        "_type": "location",
        "lat": lat,
        "lon": lon,
        "tst": tst,
        "tid": tid,
        "src": "gps-to-mqtt",
    });
    let o = doc.as_object_mut()?;
    if let Some(a) = fix.accuracy_m {
        o.insert("acc".into(), json!(a.round() as i64));
    }
    if let Some(a) = fix.altitude_m {
        o.insert("alt".into(), json!(a.round() as i64));
    }
    if let Some(v) = fix.speed_kph() {
        o.insert("vel".into(), json!(v.round() as i64));
    }
    if let Some(c) = fix.course_deg {
        o.insert("cog".into(), json!(c.round() as i64));
    }
    Some(doc.to_string())
}

/// Home Assistant MQTT discovery documents.
pub fn homeassistant_discovery(config: &AppConfig, client: &str) -> Vec<Msg> {
    let base = config.base_topic();
    let node = sanitize(
        &config
            .mqtt_client_id
            .clone()
            .unwrap_or_else(|| sanitize(base)),
    );
    let node = if node.is_empty() {
        "gps_to_mqtt".to_string()
    } else {
        node
    };
    let fix_topic = format!("{base}/fix");
    let avail = format!("{base}/status");
    let device = json!({
        "identifiers": [format!("gps_to_mqtt_{node}")],
        "name": format!("GNSS {base}"),
        "manufacturer": "G86 Racing",
        "model": "gps-to-mqtt",
        "sw_version": env!("CARGO_PKG_VERSION"),
    });
    let prefix = config.mqtt_homeassistant_prefix.trim_end_matches('/');
    let mut out = vec![Msg {
        topic: format!("{prefix}/device_tracker/{node}/config"),
        payload: json!({
            "name": "Location",
            "unique_id": format!("{node}_location"),
            "json_attributes_topic": fix_topic,
            "json_attributes_template":
                "{{ {'latitude': value_json.latitude, 'longitude': value_json.longitude, 'gps_accuracy': value_json.accuracy_m, 'altitude': value_json.altitude_m, 'speed': value_json.speed_mps, 'course': value_json.course_deg} | tojson }}",
            "source_type": "gps",
            "availability_topic": avail,
            "device": device,
            "origin": {"name": "gps-to-mqtt", "sw_version": env!("CARGO_PKG_VERSION")},
        })
        .to_string(),
        retain: true,
    }];
    // (key, name, value template, unit, device class)
    type Sensor = (
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
    );
    let sensors: &[Sensor] = &[
        (
            "speed",
            "Speed",
            "{{ ((value_json.speed_mps or 0) * 3.6) | round(1) }}",
            Some("km/h"),
            Some("speed"),
        ),
        (
            "altitude",
            "Altitude",
            "{{ value_json.altitude_m }}",
            Some("m"),
            Some("distance"),
        ),
        (
            "satellites",
            "Satellites used",
            "{{ value_json.satellites_used }}",
            None,
            None,
        ),
        ("hdop", "HDOP", "{{ value_json.hdop }}", None, None),
        (
            "accuracy",
            "Accuracy",
            "{{ value_json.accuracy_m | round(1) }}",
            Some("m"),
            Some("distance"),
        ),
        (
            "fix_quality",
            "Fix quality",
            "{{ value_json.quality }}",
            None,
            Some("enum"),
        ),
        (
            "fix_mode",
            "Fix mode",
            "{{ value_json.mode }}",
            None,
            Some("enum"),
        ),
        ("mgrs", "MGRS", "{{ value_json.mgrs }}", None, None),
    ];
    for (key, name, template, unit, class) in sensors {
        let mut doc = json!({
            "name": name,
            "unique_id": format!("{node}_{key}"),
            "state_topic": fix_topic,
            "value_template": template,
            "availability_topic": avail,
            "device": {"identifiers": [format!("gps_to_mqtt_{node}")]},
        });
        let o = doc.as_object_mut().expect("object");
        if let Some(u) = unit {
            o.insert("unit_of_measurement".into(), json!(u));
            o.insert("state_class".into(), json!("measurement"));
        }
        if let Some(c) = class {
            o.insert("device_class".into(), json!(c));
        }
        out.push(Msg {
            topic: format!("{prefix}/sensor/{node}/{key}/config"),
            payload: doc.to_string(),
            retain: true,
        });
    }
    debug!("Home Assistant discovery for node {node} (client {client})");
    out
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

// ── Runtime ──────────────────────────────────────────────────────────────

/// Per reply topic: bursts of up to DEMAND_BURST requests are answered at
/// once (a display keeps several in flight, and Wi-Fi delivers them in bursts),
/// at most DEMAND_RATE per second on average.
const DEMAND_BURST: f32 = 12.0;
const DEMAND_RATE: f32 = 100.0;
/// A request waits at most this long for a new sample.
const DEMAND_HOLD: Duration = Duration::from_secs(2);
const DEMAND_MAX_CLIENTS: usize = 32;
const DEMAND_MAX_AGE_S: f64 = 2.0;

/// Token bucket for on-demand answers to one reply topic.
#[derive(Debug, Clone, Copy)]
pub struct DemandBucket {
    tokens: f32,
    at: Instant,
}

impl DemandBucket {
    pub fn new(now: Instant) -> Self {
        DemandBucket {
            tokens: DEMAND_BURST,
            at: now,
        }
    }

    /// One answer if the bucket has a token; refills at DEMAND_RATE per second.
    pub fn take(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.at).as_secs_f32();
        self.tokens = (self.tokens + dt * DEMAND_RATE).min(DEMAND_BURST);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One display asking on demand: its rate bucket and the requests it holds
/// open. A request is answered with the next sample it hasn't had yet, so
/// a display gets every new sample once, as soon as it exists, and never a
/// duplicate (that would only cost airtime on a weak link).
#[derive(Debug, Clone, Copy)]
pub struct DemandClient {
    bucket: DemandBucket,
    pending: u8,
    last_seq: Option<u64>,
    since: Instant,
}

impl DemandClient {
    pub fn new(now: Instant) -> Self {
        DemandClient {
            bucket: DemandBucket::new(now),
            pending: 0,
            last_seq: None,
            since: now,
        }
    }

    /// A request arrived; false if over the rate (dropped).
    pub fn request(&mut self, now: Instant) -> bool {
        if !self.bucket.take(now) {
            return false;
        }
        self.pending = (self.pending + 1).min(DEMAND_BURST as u8);
        self.since = now;
        true
    }

    /// Whether to answer one held request now with sample `seq` (None: no
    /// fresh sample). Held requests expire after DEMAND_HOLD.
    pub fn ready(&mut self, seq: Option<u64>, now: Instant) -> bool {
        if self.pending == 0 {
            return false;
        }
        if now.saturating_duration_since(self.since) > DEMAND_HOLD {
            self.pending = 0;
            return false;
        }
        match seq {
            Some(s) if self.last_seq != Some(s) => {
                self.last_seq = Some(s);
                self.pending -= 1;
                true
            }
            _ => false,
        }
    }

    pub fn last_seen(&self) -> Instant {
        self.since
    }
}

/// A usable reply topic: 1–128 bytes of UTF-8, no wildcards, not a `$` system topic.
pub fn valid_reply_topic(payload: &[u8]) -> Option<&str> {
    let t = std::str::from_utf8(payload).ok()?.trim();
    (!t.is_empty() && t.len() <= 128 && !t.contains(['+', '#', '\0']) && !t.starts_with('$'))
        .then_some(t)
}

/// The fix an on-demand answer would carry now, or None (no fresh valid fix).
pub fn demand_sequence(shared: &Shared) -> Option<u64> {
    let s = shared.read();
    if s.fix_age_s(Instant::now())? > DEMAND_MAX_AGE_S {
        return None;
    }
    let fix = s.fix.as_ref()?;
    (fix.valid && fix.latitude.is_some() && fix.longitude.is_some()).then_some(fix.sequence)
}

/// The latest valid fix for an on-demand request; `None` when there is none.
pub fn demand_reply(shared: &Shared) -> Option<String> {
    let s = shared.read();
    if s.fix_age_s(Instant::now())? > DEMAND_MAX_AGE_S {
        return None;
    }
    let fix = s.fix.as_ref()?;
    let (lat, lon) = (fix.latitude?, fix.longitude?);
    if !fix.valid {
        return None;
    }
    let t = fix.telemetry.as_ref();
    // Every other numeric value of the fix, for displays that show any channel.
    let mut channels = serde_json::Map::new();
    let mut put = |name: &str, v: Option<f64>| {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            channels.insert(name.into(), json!(v));
        }
    };
    put("gpsSpeedKmh", fix.speed_mps.map(|v| v * 3.6));
    put("altitudeM", fix.altitude_m);
    put("accuracyM", fix.accuracy_m);
    put("satellitesUsed", fix.satellites_used.map(f64::from));
    put("satellitesInView", fix.satellites_in_view.map(f64::from));
    put("hdop", fix.hdop);
    put("vdop", fix.vdop);
    put("pdop", fix.pdop);
    put("rateOfTurnDps", fix.rate_of_turn_dps);
    put("combinedG", t.and_then(|t| t.combined_g));
    put("yawRateDps", t.and_then(|t| t.yaw_rate_dps));
    put("distanceM", t.map(|t| t.distance_m));
    put("maxSpeedKmh", t.map(|t| t.max_speed_mps * 3.6));
    Some(
        json!({
            "schema": 1,
            "source": "gps-to-mqtt",
            "timestampMs": fix.received_ms,
            "latitude": lat,
            "longitude": lon,
            "speedMps": fix.speed_mps,
            "headingDeg": fix.course_deg.or(fix.heading_deg),
            "lateralG": t.and_then(|t| t.lateral_g),
            "longitudinalG": t.and_then(|t| t.longitudinal_g),
            "channels": channels,
        })
        .to_string(),
    )
}

pub async fn run(config: AppConfig, shared: Arc<Shared>, cancel: CancellationToken) -> Result<()> {
    let id = client_id(&config);
    let opts = options(&config, &id)?;
    let (client, mut eventloop) = AsyncClient::new(opts, 1024);
    let (conn_tx, conn_rx) = watch::channel(0u64);
    info!("MQTT client {id} → {}", config.mqtt_address());

    // Event loop: drives the connection, reconnects with backoff.
    let el_shared = shared.clone();
    let el_cancel = cancel.clone();
    let el_client = client.clone();
    let demand_topic = format!(
        "{}/position/get",
        config.mqtt_base_topic.trim_end_matches('/')
    );
    let max_attempts = config.mqtt_reconnect_max_attempts;
    let event_loop = tokio::spawn(async move {
        let mut answered: HashMap<String, DemandClient> = HashMap::new();
        // releases held requests when a new sample exists
        let mut tick = tokio::time::interval(Duration::from_millis(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut delay = Duration::from_secs(1);
        let mut failures = 0u32;
        let mut generation = 0u64;
        el_shared.set_mqtt(LinkStatus::Connecting);
        loop {
            let polled = tokio::select! {
                r = eventloop.poll() => r,
                _ = tick.tick(), if answered.values().any(|c| c.pending > 0) => {
                    let now = Instant::now();
                    let seq = demand_sequence(&el_shared);
                    for (reply, c) in answered.iter_mut() {
                        if c.ready(seq, now)
                            && let Some(doc) = demand_reply(&el_shared)
                        {
                            match el_client.try_publish(reply.clone(), QoS::AtMostOnce, false, doc) {
                                Ok(()) => Counters::inc(&el_shared.counters.mqtt_published),
                                Err(_) => Counters::inc(&el_shared.counters.mqtt_dropped),
                            }
                        }
                    }
                    continue;
                }
                _ = el_cancel.cancelled() => {
                    // Give the offline status + DISCONNECT a moment to flush.
                    let _ = tokio::time::timeout(Duration::from_secs(1), async {
                        while eventloop.poll().await.is_ok() {}
                    }).await;
                    break;
                }
            };
            match polled {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    info!("MQTT connected ({:?})", ack.code);
                    let _ = el_client.try_subscribe(demand_topic.clone(), QoS::AtMostOnce);
                    delay = Duration::from_secs(1);
                    failures = 0;
                    generation += 1;
                    let _ = conn_tx.send(generation);
                    el_shared.set_mqtt(LinkStatus::Connected);
                }
                Ok(Event::Incoming(Packet::Publish(p))) if p.topic == demand_topic => {
                    let Some(reply) = valid_reply_topic(&p.payload) else {
                        continue;
                    };
                    let now = Instant::now();
                    if answered.len() >= DEMAND_MAX_CLIENTS && !answered.contains_key(reply) {
                        answered.retain(|_, c| {
                            now.duration_since(c.last_seen()) < Duration::from_secs(10)
                        });
                        if answered.len() >= DEMAND_MAX_CLIENTS {
                            continue;
                        }
                    }
                    let c = answered
                        .entry(reply.to_string())
                        .or_insert_with(|| DemandClient::new(now));
                    if !c.request(now) {
                        continue;
                    }
                    // answered at once if there is a sample it hasn't had, else held for the next one
                    if c.ready(demand_sequence(&el_shared), now)
                        && let Some(doc) = demand_reply(&el_shared)
                    {
                        match el_client.try_publish(reply, QoS::AtMostOnce, false, doc) {
                            Ok(()) => Counters::inc(&el_shared.counters.mqtt_published),
                            Err(_) => Counters::inc(&el_shared.counters.mqtt_dropped),
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = conn_tx.send(0);
                    el_shared.set_mqtt(LinkStatus::Disconnected);
                    failures += 1;
                    if max_attempts > 0 && failures >= max_attempts {
                        error!("MQTT: giving up after {failures} attempts: {e}");
                        break;
                    }
                    warn!("MQTT connection error: {e}; retrying in {delay:?}");
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = el_cancel.cancelled() => break,
                    }
                    delay = (delay * 2).min(RECONNECT_MAX);
                    el_shared.set_mqtt(LinkStatus::Connecting);
                }
            }
        }
        el_shared.set_mqtt(LinkStatus::Disconnected);
    });

    let mut publisher = Publisher::new(config, client, shared.clone(), id);
    publisher.run(shared.bus.subscribe(), conn_rx, cancel).await;
    let _ = event_loop.await;
    Ok(())
}

struct Publisher {
    config: AppConfig,
    client: AsyncClient,
    shared: Arc<Shared>,
    client_id: String,
    base: String,
    qos: QoS,
    last: HashMap<String, String>,
    sat_topics: HashSet<String>,
    min_interval: Option<Duration>,
    last_state_publish: Option<Instant>,
    latest_fix: Option<Arc<Fix>>,
    latest_sky: Option<Arc<Sky>>,
}

impl Publisher {
    fn new(config: AppConfig, client: AsyncClient, shared: Arc<Shared>, client_id: String) -> Self {
        let min_interval = (config.mqtt_max_rate_hz > 0.0)
            .then(|| Duration::from_secs_f64(1.0 / config.mqtt_max_rate_hz));
        Self {
            base: config.base_topic().to_string(),
            qos: if config.mqtt_qos == 1 {
                QoS::AtLeastOnce
            } else {
                QoS::AtMostOnce
            },
            config,
            client,
            shared,
            client_id,
            last: HashMap::new(),
            sat_topics: HashSet::new(),
            min_interval,
            last_state_publish: None,
            latest_fix: None,
            latest_sky: None,
        }
    }

    fn send(&self, topic: String, payload: String, retain: bool, qos: QoS) -> bool {
        match self.client.try_publish(topic, qos, retain, payload) {
            Ok(()) => {
                Counters::inc(&self.shared.counters.mqtt_published);
                true
            }
            Err(_) => {
                Counters::inc(&self.shared.counters.mqtt_dropped);
                false
            }
        }
    }

    /// Retained value with change detection.
    fn state(&mut self, suffix: &str, payload: String) {
        let topic = format!("{}{}", self.base, suffix);
        if self.last.get(&topic) == Some(&payload) {
            return;
        }
        if self.send(
            topic.clone(),
            payload.clone(),
            self.config.mqtt_retain,
            self.qos,
        ) {
            self.last.insert(topic, payload);
        }
    }

    fn on_connect(&mut self) {
        self.last.clear();
        self.send(
            format!("{}/status", self.base),
            "online".into(),
            true,
            QoS::AtLeastOnce,
        );
        if self.config.wants(MqttFormat::Homeassistant) {
            for m in homeassistant_discovery(&self.config, &self.client_id) {
                self.send(m.topic, m.payload, m.retain, QoS::AtLeastOnce);
            }
        }
        // Re-send retained state immediately instead of waiting for the next fix.
        if let Some(fix) = self.latest_fix.clone() {
            self.publish_fix(&fix);
        }
        if let Some(sky) = self.latest_sky.clone() {
            self.publish_sky(&sky);
        }
    }

    fn publish_fix(&mut self, fix: &Fix) {
        if self.config.wants(MqttFormat::Legacy) {
            for (suffix, payload) in legacy_fix(fix) {
                self.state(suffix, payload);
            }
        }
        if (self.config.wants(MqttFormat::Json) || self.config.wants(MqttFormat::Homeassistant))
            && let Ok(doc) = serde_json::to_string(fix)
        {
            let topic = format!("{}/fix", self.base);
            self.send(topic, doc, self.config.mqtt_retain, self.qos);
        }
        if self.config.wants(MqttFormat::Owntracks)
            && let Some(doc) = owntracks(fix, &self.config.mqtt_owntracks_tid)
        {
            let topic = self.config.mqtt_owntracks_topic.clone().unwrap_or_else(|| {
                // Stable across restarts (the generated client ID is not).
                format!("owntracks/gps-to-mqtt/{}", sanitize(&self.base))
            });
            self.send(topic, doc, true, self.qos);
        }
    }

    fn publish_sky(&mut self, sky: &Sky) {
        if self.config.wants(MqttFormat::Legacy) {
            let current: HashSet<String> = legacy_sky(sky)
                .into_iter()
                .map(|(suffix, payload)| {
                    self.state(&suffix, payload);
                    suffix
                })
                .collect();
            // Clear retained topics of satellites that left the sky.
            for gone in self.sat_topics.difference(&current) {
                let topic = format!("{}{}", self.base, gone);
                self.last.remove(&topic);
                self.send(topic, String::new(), true, self.qos);
            }
            self.sat_topics = current;
        }
        if (self.config.wants(MqttFormat::Json) || self.config.wants(MqttFormat::Homeassistant))
            && let Ok(doc) = serde_json::to_string(sky)
        {
            self.send(
                format!("{}/sky", self.base),
                doc,
                self.config.mqtt_retain,
                self.qos,
            );
        }
    }

    async fn run(
        &mut self,
        mut bus: broadcast::Receiver<BusEvent>,
        mut conn: watch::Receiver<u64>,
        cancel: CancellationToken,
    ) {
        let mut connected = false;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                changed = conn.changed() => {
                    if changed.is_err() { break; }
                    let generation = *conn.borrow_and_update();
                    connected = generation > 0;
                    if connected { self.on_connect(); }
                }
                event = bus.recv() => match event {
                    Ok(event) => self.on_event(event, connected),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        self.shared.counters.mqtt_dropped.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        if connected {
            self.send(
                format!("{}/status", self.base),
                "offline".into(),
                true,
                QoS::AtLeastOnce,
            );
            let _ = self.client.try_disconnect();
        }
    }

    fn on_event(&mut self, event: BusEvent, connected: bool) {
        match event {
            BusEvent::Fix(fix) => {
                self.latest_fix = Some(fix.clone());
                if !connected {
                    return Counters::inc(&self.shared.counters.mqtt_dropped);
                }
                let now = Instant::now();
                if let (Some(min), Some(last)) = (self.min_interval, self.last_state_publish)
                    && now.duration_since(last) < min
                {
                    return;
                }
                self.last_state_publish = Some(now);
                self.publish_fix(&fix);
            }
            BusEvent::Sky(sky) => {
                self.latest_sky = Some(sky.clone());
                if connected {
                    self.publish_sky(&sky);
                }
            }
            BusEvent::G86(packet) if self.config.wants(MqttFormat::G86) => {
                if connected {
                    // Never retained: G86 rejects retained frames as stale.
                    self.send(
                        format!("{}/position", self.base),
                        packet.to_string(),
                        false,
                        QoS::AtMostOnce,
                    );
                } else {
                    Counters::inc(&self.shared.counters.mqtt_dropped);
                }
            }
            BusEvent::G86(_) | BusEvent::Nmea(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FixQuality, Satellite, Telemetry};
    use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};

    fn fix() -> Fix {
        Fix {
            valid: true,
            quality: FixQuality::Gps,
            latitude: Some(56.966_666_7),
            longitude: Some(24.233_333_3),
            altitude_m: Some(12.04),
            speed_mps: Some(25.0),
            course_deg: Some(85.0),
            satellites_used: Some(12),
            satellites_in_view: Some(20),
            hdop: Some(0.8),
            vdop: Some(1.234),
            utc_time: NaiveTime::from_hms_milli_opt(12, 0, 0, 100),
            utc_date: NaiveDate::from_ymd_opt(2026, 9, 23),
            time: Some(Utc.timestamp_millis_opt(1_790_164_800_100).unwrap()),
            accuracy_m: Some(4.0),
            accuracy_source: Some(AccuracySource::HdopEstimate),
            mgrs: Some("35V LC 12345 67890".into()),
            mgrs_8: Some("35V LC 1234 6789".into()),
            telemetry: Some(Telemetry {
                longitudinal_accel_mps2: Some(1.23456),
                max_speed_mps: 30.0,
                distance_m: 100.04,
                ..Telemetry::default()
            }),
            ..Fix::default()
        }
    }

    #[test]
    fn demand_client_answers_each_sample_once() {
        let t0 = Instant::now();
        let mut c = DemandClient::new(t0);
        for _ in 0..3 {
            assert!(c.request(t0));
        }
        assert!(c.ready(Some(1), t0), "a sample it hasn't had: at once");
        assert!(!c.ready(Some(1), t0), "the same sample again: held");
        assert!(!c.ready(None, t0), "no fresh data: held");
        assert!(
            c.ready(Some(2), t0 + Duration::from_millis(20)),
            "the next sample releases one"
        );
        assert!(c.ready(Some(3), t0 + Duration::from_millis(40)));
        assert!(
            !c.ready(Some(4), t0 + Duration::from_millis(60)),
            "all three answered"
        );
        // held requests expire
        assert!(c.request(t0 + Duration::from_millis(100)));
        assert!(!c.ready(
            Some(4),
            t0 + Duration::from_millis(100) + DEMAND_HOLD + Duration::from_millis(1)
        ));
        assert!(!c.ready(Some(5), t0 + Duration::from_secs(10)));
    }

    #[test]
    fn demand_bucket_allows_bursts_and_caps_the_rate() {
        let t0 = Instant::now();
        let mut b = DemandBucket::new(t0);
        // a burst of in-flight requests arriving together is answered in full
        for _ in 0..12 {
            assert!(b.take(t0));
        }
        assert!(!b.take(t0), "beyond the burst");
        // refills at 100 per second: one more after 10 ms
        assert!(b.take(t0 + Duration::from_millis(10)));
        assert!(!b.take(t0 + Duration::from_millis(11)));
        // a second of requests every millisecond: about 100 answered
        let mut n = 0;
        for ms in 0..1000u64 {
            if b.take(t0 + Duration::from_millis(20 + ms)) {
                n += 1;
            }
        }
        assert!((95..=110).contains(&n), "{n} answered");
    }

    #[test]
    fn demand_reply_topics_are_validated() {
        assert_eq!(
            valid_reply_topic(b"/GOLF86/DISPLAY/x/gps"),
            Some("/GOLF86/DISPLAY/x/gps")
        );
        for bad in [&b""[..], b"a/+", b"#", b"$SYS/x"] {
            assert_eq!(valid_reply_topic(bad), None);
        }
    }

    #[test]
    fn demand_reply_needs_a_fresh_valid_fix() {
        use crate::shared::Info;
        let shared = Shared::new(Info::default(), true);
        assert_eq!(demand_reply(&shared), None);
        {
            let mut s = shared.write();
            s.fix = Some(Arc::new(fix()));
            s.last_fix_at = Some(Instant::now());
        }
        let doc: serde_json::Value =
            serde_json::from_str(&demand_reply(&shared).expect("fresh fix")).unwrap();
        assert_eq!(doc["schema"], 1);
        assert!(doc["latitude"].is_number() && doc["longitude"].is_number());
        assert_eq!(doc["channels"]["altitudeM"], 12.04);
        assert_eq!(doc["channels"]["hdop"], 0.8);
        assert!(doc["channels"]["gpsSpeedKmh"].is_number());
        shared.write().last_fix_at = Some(Instant::now() - Duration::from_secs(5));
        assert_eq!(demand_reply(&shared), None, "a stale fix is not answered");
    }

    #[test]
    fn legacy_topics_keep_0_6_formats() {
        let m: HashMap<_, _> = legacy_fix(&fix()).into_iter().collect();
        assert_eq!(m["/LAT"], "56.9666667");
        assert_eq!(m["/ALT"], "12.0");
        assert_eq!(m["/SPD"], "90.0");
        assert_eq!(m["/SPD_KPH"], "90.0");
        assert_eq!(m["/SPD_MPH"], "55.9");
        assert_eq!(m["/SPD_KTS"], "48.60");
        assert_eq!(m["/CRS"], "85");
        assert_eq!(m["/QTY"], "1");
        assert_eq!(m["/HDOP"], "0.8");
        assert_eq!(m["/VDOP"], "1.23");
        assert_eq!(m["/TME"], "12:00:00.100");
        assert_eq!(m["/DTE"], "2026-09-23");
        assert_eq!(m["/ACCEL_LONG_MPS2"], "1.235");
        assert_eq!(m["/ACCEL_LAT_G"], "0.000");
        assert_eq!(m["/MAX_SPEED"], "108.0");
        assert_eq!(m["/DISTANCE"], "100.0");
        assert_eq!(m["/BRAKING"], "0");
        assert!(
            !m.contains_key("/POSITION_ACCURACY"),
            "only GST accuracy is published there"
        );
        assert!(!m.contains_key("/LAP_TIME"));
        assert_eq!(m["/MGRS"], "35V LC 12345 67890");
        assert_eq!(m["/MGRS_8"], "35V LC 1234 6789");
    }

    #[test]
    fn invalid_fix_publishes_quality_but_no_position() {
        let mut f = fix();
        f.valid = false;
        f.quality = FixQuality::Invalid;
        f.speed_mps = Some(0.5);
        let m: HashMap<_, _> = legacy_fix(&f).into_iter().collect();
        assert_eq!(m["/QTY"], "0");
        assert!(!m.contains_key("/LAT"));
        assert!(!m.contains_key("/CRS"), "course suppressed below 3 km/h");
    }

    #[test]
    fn legacy_satellite_payload_matches_dashboard_regex() {
        let sky = Sky {
            satellites: vec![Satellite {
                constellation: Constellation::Galileo,
                prn: 3,
                elevation_deg: Some(45),
                azimuth_deg: Some(120),
                snr_dbhz: Some(40),
                used: true,
            }],
            ..Sky::default()
        };
        assert_eq!(
            legacy_sky(&sky),
            [(
                "/SAT/VEHICLES/E03".to_string(),
                "PRN: 3, Type: Galileo, Elevation: 45, Azimuth: 120, SNR: 40, In View: true"
                    .to_string()
            )]
        );
    }

    #[test]
    fn owntracks_document() {
        let doc: serde_json::Value =
            serde_json::from_str(&owntracks(&fix(), "GP").unwrap()).unwrap();
        assert_eq!(doc["_type"], "location");
        assert_eq!(doc["tst"], 1_790_164_800);
        assert_eq!(doc["vel"], 90);
        assert_eq!(doc["acc"], 4);
        let mut f = fix();
        f.valid = false;
        assert!(owntracks(&f, "GP").is_none());
    }

    #[test]
    fn homeassistant_discovery_documents() {
        let mut c = AppConfig::default();
        c.mqtt_base_topic = "/CAR/GPS/".into();
        let msgs = homeassistant_discovery(&c, "id");
        assert_eq!(msgs[0].topic, "homeassistant/device_tracker/car_gps/config");
        let tracker: serde_json::Value = serde_json::from_str(&msgs[0].payload).unwrap();
        assert_eq!(tracker["json_attributes_topic"], "/CAR/GPS/fix");
        assert_eq!(tracker["availability_topic"], "/CAR/GPS/status");
        assert!(msgs.iter().all(|m| m.retain));
        assert!(
            msgs.iter()
                .any(|m| m.topic == "homeassistant/sensor/car_gps/speed/config")
        );
    }

    #[test]
    fn fix_json_uses_explicit_units() {
        let v: serde_json::Value = serde_json::to_value(fix()).unwrap();
        assert_eq!(v["speed_mps"], 25.0);
        assert_eq!(v["quality"], "gps");
        assert_eq!(v["accuracy_source"], "hdop-estimate");
        assert_eq!(v["time"], "2026-09-23T12:00:00.100Z");
        assert!(v.get("utc_time").is_none());
    }
}
