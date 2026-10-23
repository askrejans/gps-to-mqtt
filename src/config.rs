//! Configuration: TOML files + `GPS_TO_MQTT_*` environment variables.
//!
//! Priority (highest → lowest):
//! 1. `GPS_TO_MQTT_<KEY>` environment variables (e.g. `GPS_TO_MQTT_MQTT_HOST`)
//! 2. `--config <file>`
//! 3. Default search paths (`/etc/gps-to-mqtt/settings.toml`, exe dir, `./`)
//! 4. Built-in defaults
//!
//! Keys are flat (`mqtt_host`, not `[mqtt] host`) so every setting maps 1:1 to
//! an environment variable. List settings accept a TOML array or, from the
//! environment, a delimited string (`mqtt_formats = "legacy,json"`,
//! `init_commands = "PMTK220,100;PMTK314,0,1,0,1,1,5,0,0,0,0,0,0,0,0,0,0,0,0,0"`).

use anyhow::{Context, Result, bail};
use config::{Config, Environment, File};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

pub const VALID_BAUD_RATES: &[u32] = &[
    4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    /// Local USB / UART receiver
    Serial,
    /// TCP client to a serial-to-TCP bridge (io-to-net, ser2net, receiver Wi-Fi module)
    Tcp,
    /// NMEA datagrams (marine multiplexers, phone apps, OpenCPN, NMEA routers)
    Udp,
    /// Existing gpsd daemon (raw NMEA via `?WATCH`)
    Gpsd,
    /// Replay of a recorded NMEA log
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiverProfile {
    /// Any NMEA-0183 receiver; nothing is written to the device.
    Generic,
    /// u-blox M8/M9/M10/F9 (UBX-CFG-RATE)
    Ublox,
    /// MediaTek MT33xx and derivatives (PMTK220): GlobalTop, Adafruit, Quectel L76/L80
    Mediatek,
    /// Quectel LC29H/LC79H/LG69T (PAIR050)
    Quectel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MqttFormat {
    /// One retained scalar topic per value (`<base>/LAT`, `<base>/SPD` …) — G86 dashboard
    Legacy,
    /// `<base>/fix` and `<base>/sky` JSON documents
    Json,
    /// `<base>/position` canonical GPS packet (schema 1).
    #[serde(rename = "canonical", alias = "g86")]
    G86,
    /// Home Assistant MQTT discovery (device_tracker + sensors, uses JSON topics)
    Homeassistant,
    /// OwnTracks location messages (Home Assistant, Traccar, OwnTracks Recorder)
    Owntracks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // ── Source ───────────────────────────────────────────────────────────
    #[serde(default = "default_connection_type")]
    pub connection_type: ConnectionType,
    /// Serial device path, or `auto` to detect a GNSS receiver on USB serial ports.
    #[serde(default = "default_port_name")]
    pub port_name: String,
    /// Serial baud rate; `0` = auto-detect.
    #[serde(default)]
    pub baud_rate: u32,
    pub tcp_host: Option<String>,
    pub tcp_port: Option<u16>,
    /// UDP listen address for `connection_type = "udp"`.
    #[serde(default = "default_udp_bind")]
    pub udp_bind: String,
    /// NMEA log for `connection_type = "file"`.
    pub replay_file: Option<String>,
    /// Replay speed multiplier (1.0 = real time, 0 = as fast as possible).
    #[serde(default = "default_replay_speed")]
    pub replay_speed: f64,
    #[serde(default)]
    pub replay_loop: bool,
    /// Reject sentences without a `*hh` checksum.
    #[serde(default = "default_true")]
    pub nmea_require_checksum: bool,

    // ── Receiver ─────────────────────────────────────────────────────────
    #[serde(default = "default_receiver")]
    pub receiver: ReceiverProfile,
    /// Navigation rate to configure (1–25 Hz). Unset = leave the device alone.
    pub update_rate_hz: Option<u32>,
    /// Legacy switch: equivalent to `receiver = "ublox"` + `update_rate_hz = 10`.
    #[serde(default)]
    pub set_gps_to_10hz: bool,
    /// Extra commands written after connecting. `PMTK220,100` gets `$`/checksum
    /// added; `hex:B5 62 …` is sent as raw bytes.
    #[serde(default, deserialize_with = "list_semicolon")]
    pub init_commands: Vec<String>,

    // ── MQTT ─────────────────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub mqtt_enabled: bool,
    #[serde(default = "default_mqtt_host")]
    pub mqtt_host: String,
    #[serde(default = "default_mqtt_port")]
    pub mqtt_port: u16,
    /// Client ID; auto-generated (unique per process) when unset.
    pub mqtt_client_id: Option<String>,
    /// Explicit base topic. Unset: `/g86/<car>/gps` when `car` is set, else
    /// the legacy `/GOLF86/GPS`.
    #[serde(default)]
    pub mqtt_base_topic: Option<String>,
    /// Car slug (`golf86`): selects the G86 car-computer namespace `/g86/<car>/gps`.
    #[serde(default)]
    pub car: Option<String>,
    /// `0` = retry forever.
    #[serde(default)]
    pub mqtt_reconnect_max_attempts: u32,
    pub mqtt_username: Option<String>,
    pub mqtt_password: Option<String>,
    /// Read the password from a file (Docker/systemd secrets). Overrides `mqtt_password`.
    pub mqtt_password_file: Option<String>,
    #[serde(default)]
    pub mqtt_use_tls: bool,
    /// PEM CA bundle for TLS; system roots (webpki) when unset.
    pub mqtt_ca_file: Option<String>,
    /// PEM client certificate + key for mutual TLS.
    pub mqtt_client_cert_file: Option<String>,
    pub mqtt_client_key_file: Option<String>,
    #[serde(default = "default_keep_alive")]
    pub mqtt_keep_alive_s: u64,
    /// QoS for data topics: 0 or 1.
    #[serde(default)]
    pub mqtt_qos: u8,
    /// Retain flag for legacy/JSON state topics.
    #[serde(default = "default_true")]
    pub mqtt_retain: bool,
    #[serde(default = "default_mqtt_formats", deserialize_with = "list_formats")]
    pub mqtt_formats: Vec<MqttFormat>,
    /// Cap for legacy/JSON/OwnTracks publishing (Hz); `0` = every fix. G86 packets are never throttled.
    #[serde(default)]
    pub mqtt_max_rate_hz: f64,
    #[serde(default = "default_ha_prefix")]
    pub mqtt_homeassistant_prefix: String,
    /// OwnTracks topic; default `owntracks/gps-to-mqtt/<client_id>`.
    pub mqtt_owntracks_topic: Option<String>,
    /// OwnTracks two-character tracker ID.
    #[serde(default = "default_owntracks_tid")]
    pub mqtt_owntracks_tid: String,

    // ── HTTP API / Prometheus ────────────────────────────────────────────
    #[serde(default, alias = "prometheus_enabled")]
    pub http_enabled: bool,
    #[serde(default = "default_http_bind", alias = "prometheus_bind")]
    pub http_bind: String,
    #[serde(default = "default_http_port", alias = "prometheus_port")]
    pub http_port: u16,
    /// Bearer token required for `/api/*` (health and metrics stay open).
    pub http_token: Option<String>,
    /// `Access-Control-Allow-Origin` value for browser dashboards (unset = no CORS).
    pub http_cors_origin: Option<String>,
    /// Serve the built-in web dashboard at `/` (requires `http_enabled`).
    #[serde(default = "default_true")]
    pub http_ui: bool,
    /// Map tiles behind the dashboard track (`{z}/{x}/{y}` template). Empty =
    /// no map and no third-party requests. Tile servers learn the viewed area.
    #[serde(default = "default_map_tiles")]
    pub http_map_tiles: String,
    /// Render map tiles dark (colour-inverted). Turn off for natively dark tile sets.
    #[serde(default = "default_true")]
    pub http_map_dark: bool,
    /// Attribution shown on the map (required by OpenStreetMap's licence).
    #[serde(default = "default_map_attribution")]
    pub http_map_attribution: String,

    // ── Network re-broadcast ─────────────────────────────────────────────
    /// NMEA-0183 TCP server, e.g. `0.0.0.0:10110` (OpenCPN, Signal K, chart plotters).
    pub nmea_tcp_listen: Option<String>,
    /// NMEA-0183 UDP target, e.g. `255.255.255.255:10110` (broadcast) or `192.168.1.20:10110`.
    pub nmea_udp_target: Option<String>,
    /// gpsd-compatible JSON server, e.g. `127.0.0.1:2947` (cgps, gpsmon, Navit, foxtrotgps …).
    pub gpsd_listen: Option<String>,
    /// Maximum concurrent clients per TCP server.
    #[serde(default = "default_max_clients")]
    pub max_clients: usize,

    // ── Processing ───────────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub telemetry_enabled: bool,
    #[serde(default = "default_telemetry_smoothing_window")]
    pub telemetry_smoothing_window: usize,
    /// Pin the reported position while stationary (removes idle jitter from
    /// every output except the raw G86 packet).
    #[serde(default)]
    pub static_hold: bool,
    /// Hold engages below this speed.
    #[serde(default = "default_hold_speed")]
    pub static_hold_speed_kph: f64,
    /// Hold releases when the raw position leaves this radius around the held point.
    #[serde(default = "default_hold_radius")]
    pub static_hold_radius_m: f64,
    /// Drop satellites not reported for this long (seconds).
    #[serde(default = "default_satellite_timeout")]
    pub satellite_timeout_s: u64,
    /// Absolute path for the synced canonical NDJSON archive of G86 packets.
    pub telemetry_log: Option<String>,

    // ── Logging / TUI ────────────────────────────────────────────────────
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub log_json: bool,
    #[serde(default = "default_tui_refresh_rate_ms")]
    pub tui_refresh_rate_ms: u64,
    #[serde(default = "default_max_log_buffer_size")]
    pub max_log_buffer_size: usize,

    /// Path of the file that was loaded (runtime only).
    #[serde(skip)]
    pub config_path: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_connection_type() -> ConnectionType {
    ConnectionType::Serial
}
fn default_port_name() -> String {
    "auto".into()
}
fn default_udp_bind() -> String {
    "0.0.0.0:10110".into()
}
fn default_replay_speed() -> f64 {
    1.0
}
fn default_receiver() -> ReceiverProfile {
    ReceiverProfile::Generic
}
fn default_mqtt_host() -> String {
    "localhost".into()
}
fn default_mqtt_port() -> u16 {
    1883
}
/// Base topic when neither `mqtt_base_topic` nor `car` is set (unchanged since 0.5).
pub const LEGACY_BASE_TOPIC: &str = "/GOLF86/GPS";

/// G86 car slug: `^[a-z0-9][a-z0-9-]{0,31}$`.
pub fn valid_car(car: &str) -> bool {
    let b = car.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}
fn default_keep_alive() -> u64 {
    30
}
fn default_mqtt_formats() -> Vec<MqttFormat> {
    vec![MqttFormat::Legacy, MqttFormat::Json, MqttFormat::G86]
}
fn default_ha_prefix() -> String {
    "homeassistant".into()
}
fn default_owntracks_tid() -> String {
    "GP".into()
}
fn default_http_bind() -> String {
    "127.0.0.1".into()
}
fn default_http_port() -> u16 {
    9090
}
fn default_map_tiles() -> String {
    "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into()
}
fn default_map_attribution() -> String {
    "© OpenStreetMap contributors".into()
}
fn default_max_clients() -> usize {
    16
}
fn default_telemetry_smoothing_window() -> usize {
    3
}
fn default_hold_speed() -> f64 {
    2.0
}
fn default_hold_radius() -> f64 {
    5.0
}
fn default_satellite_timeout() -> u64 {
    10
}
fn default_log_level() -> String {
    "info".into()
}
fn default_tui_refresh_rate_ms() -> u64 {
    100
}
fn default_max_log_buffer_size() -> usize {
    1000
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrList {
    One(String),
    Many(Vec<String>),
}

fn split_list<'de, D: Deserializer<'de>>(d: D, sep: &[char]) -> Result<Vec<String>, D::Error> {
    Ok(match StringOrList::deserialize(d)? {
        StringOrList::One(s) => s
            .split(sep)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        StringOrList::Many(v) => v,
    })
}

fn list_semicolon<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    split_list(d, &[';', '\n'])
}

fn list_formats<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<MqttFormat>, D::Error> {
    split_list(d, &[',', ';', ' '])?
        .into_iter()
        .map(|s| {
            MqttFormat::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(
                s.to_lowercase(),
            ))
        })
        .collect()
}

impl Default for AppConfig {
    fn default() -> Self {
        // Every field has a serde default, so an empty document is a valid config.
        serde_json::from_str("{}").expect("defaults deserialize")
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self
            .telemetry_log
            .as_ref()
            .is_some_and(|p| !Path::new(p).is_absolute())
        {
            bail!("telemetry_log must be an absolute path");
        }
        match self.connection_type {
            ConnectionType::Serial => {
                if self.port_name.is_empty() {
                    bail!("port_name must not be empty (use \"auto\" to detect)");
                }
                if self.baud_rate != 0 && !VALID_BAUD_RATES.contains(&self.baud_rate) {
                    bail!("baud_rate must be 0 (auto) or one of {VALID_BAUD_RATES:?}");
                }
            }
            ConnectionType::Tcp => {
                if self.tcp_host.as_deref().unwrap_or("").is_empty() {
                    bail!("tcp_host must be set when connection_type = tcp");
                }
                if self.tcp_port.unwrap_or(0) == 0 {
                    bail!("tcp_port must be set and > 0 when connection_type = tcp");
                }
            }
            ConnectionType::Udp => {
                self.udp_bind
                    .parse::<std::net::SocketAddr>()
                    .context("udp_bind must be an address such as 0.0.0.0:10110")?;
            }
            ConnectionType::Gpsd => {}
            ConnectionType::File => {
                if self.replay_file.as_deref().unwrap_or("").is_empty() {
                    bail!("replay_file must be set when connection_type = file");
                }
                if !(self.replay_speed.is_finite() && self.replay_speed >= 0.0) {
                    bail!("replay_speed must be >= 0");
                }
            }
        }
        if let Some(rate) = self.effective_update_rate() {
            if !(1..=25).contains(&rate) {
                bail!("update_rate_hz must be between 1 and 25");
            }
            if self.effective_receiver() == ReceiverProfile::Generic {
                bail!("update_rate_hz requires receiver = ublox | mediatek | quectel");
            }
        }
        if self.mqtt_enabled {
            if self.mqtt_host.is_empty() {
                bail!("mqtt_host must not be empty when mqtt_enabled = true");
            }
            if self.mqtt_port == 0 {
                bail!("mqtt_port must be > 0");
            }
            if let Some(base) = &self.mqtt_base_topic {
                if base.trim_matches('/').is_empty() {
                    bail!("mqtt_base_topic must not be empty");
                }
                if base.contains(['#', '+']) {
                    bail!("mqtt_base_topic must not contain MQTT wildcards");
                }
            }
            if self.mqtt_qos > 1 {
                bail!("mqtt_qos must be 0 or 1");
            }
            if self.mqtt_formats.is_empty() {
                bail!(
                    "mqtt_formats must list at least one of legacy, json, canonical (or g86), homeassistant, owntracks"
                );
            }
            if !(self.mqtt_max_rate_hz.is_finite() && self.mqtt_max_rate_hz >= 0.0) {
                bail!("mqtt_max_rate_hz must be >= 0");
            }
            if self.mqtt_client_cert_file.is_some() != self.mqtt_client_key_file.is_some() {
                bail!("mqtt_client_cert_file and mqtt_client_key_file must be set together");
            }
            if self.mqtt_owntracks_tid.len() > 2 {
                bail!("mqtt_owntracks_tid must be at most 2 characters");
            }
        }
        if let Some(car) = &self.car
            && !valid_car(car)
        {
            bail!(
                "car must match ^[a-z0-9][a-z0-9-]{{0,31}}$ (lower-case letters, digits, hyphens)"
            );
        }
        let levels = ["trace", "debug", "info", "warn", "error"];
        if !levels.contains(&self.log_level.to_lowercase().as_str()) {
            bail!("log_level must be one of: {levels:?}");
        }
        if self.http_enabled {
            if self.http_port == 0 {
                bail!("http_port must be > 0 when http_enabled = true");
            }
            self.http_bind
                .parse::<std::net::IpAddr>()
                .context("http_bind must be an IP address such as 127.0.0.1 or 0.0.0.0")?;
        }
        if !self.http_map_tiles.is_empty() {
            map_origin(&self.http_map_tiles)?;
        }
        for (key, value) in [
            ("nmea_tcp_listen", &self.nmea_tcp_listen),
            ("nmea_udp_target", &self.nmea_udp_target),
            ("gpsd_listen", &self.gpsd_listen),
        ] {
            if let Some(v) = value {
                v.parse::<std::net::SocketAddr>()
                    .with_context(|| format!("{key} must be an address such as 0.0.0.0:10110"))?;
            }
        }
        if self.max_clients == 0 {
            bail!("max_clients must be > 0");
        }
        if !(self.static_hold_speed_kph.is_finite()
            && (0.1..=20.0).contains(&self.static_hold_speed_kph))
        {
            bail!("static_hold_speed_kph must be between 0.1 and 20");
        }
        if !(self.static_hold_radius_m.is_finite()
            && (0.5..=100.0).contains(&self.static_hold_radius_m))
        {
            bail!("static_hold_radius_m must be between 0.5 and 100");
        }
        if self.satellite_timeout_s == 0 {
            bail!("satellite_timeout_s must be > 0");
        }
        for cmd in &self.init_commands {
            crate::receiver::encode_command(cmd)
                .with_context(|| format!("invalid init_commands entry {cmd:?}"))?;
        }
        Ok(())
    }

    /// Settings as JSON with secrets masked (for `--check-config` and `/api/v1/config`).
    pub fn redacted(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        for key in ["mqtt_password", "http_token"] {
            if v.get(key).is_some_and(|x| !x.is_null()) {
                v[key] = "***".into();
            }
        }
        v
    }

    pub fn effective_receiver(&self) -> ReceiverProfile {
        if self.set_gps_to_10hz && self.receiver == ReceiverProfile::Generic {
            ReceiverProfile::Ublox
        } else {
            self.receiver
        }
    }

    pub fn effective_update_rate(&self) -> Option<u32> {
        self.update_rate_hz.or(self.set_gps_to_10hz.then_some(10))
    }

    /// Resolved base topic without trailing slash (`/CAR/GPS/` → `/CAR/GPS`):
    /// explicit `mqtt_base_topic`, else `/g86/<car>/gps`, else the legacy default.
    pub fn base_topic(&self) -> String {
        match (&self.mqtt_base_topic, &self.car) {
            (Some(base), _) => base.trim_end_matches('/').to_string(),
            (None, Some(car)) => format!("/g86/{car}/gps"),
            (None, None) => LEGACY_BASE_TOPIC.to_string(),
        }
    }

    /// Where the base topic came from, for the startup log.
    pub fn base_topic_origin(&self) -> &'static str {
        match (&self.mqtt_base_topic, &self.car) {
            (Some(_), _) => "mqtt_base_topic",
            (None, Some(_)) => "car",
            (None, None) => "legacy default",
        }
    }

    pub fn wants(&self, format: MqttFormat) -> bool {
        self.mqtt_formats.contains(&format)
    }

    pub fn connection_display(&self) -> String {
        match self.connection_type {
            ConnectionType::Serial => {
                let baud = if self.baud_rate == 0 {
                    "auto".to_string()
                } else {
                    self.baud_rate.to_string()
                };
                format!("{} @ {} baud", self.port_name, baud)
            }
            ConnectionType::Tcp => format!(
                "TCP {}:{}",
                self.tcp_host.as_deref().unwrap_or("?"),
                self.tcp_port.unwrap_or(0)
            ),
            ConnectionType::Udp => format!("UDP {}", self.udp_bind),
            ConnectionType::Gpsd => format!("gpsd {}", self.gpsd_address()),
            ConnectionType::File => {
                format!("replay {}", self.replay_file.as_deref().unwrap_or("?"))
            }
        }
    }

    pub fn gpsd_address(&self) -> String {
        format!(
            "{}:{}",
            self.tcp_host.as_deref().unwrap_or("127.0.0.1"),
            self.tcp_port.unwrap_or(2947)
        )
    }

    pub fn mqtt_address(&self) -> String {
        format!("{}:{}", self.mqtt_host, self.mqtt_port)
    }

    /// Password from `mqtt_password_file` (trimmed) or `mqtt_password`.
    pub fn mqtt_password(&self) -> Result<Option<String>> {
        match &self.mqtt_password_file {
            Some(path) => Ok(Some(
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading mqtt_password_file {path}"))?
                    .trim_end_matches(['\r', '\n'])
                    .to_string(),
            )),
            None => Ok(self.mqtt_password.clone()),
        }
    }
}

/// Validate a tile URL template and return its origin (`https://host[:port]`)
/// for the dashboard's Content-Security-Policy.
pub fn map_origin(template: &str) -> Result<String> {
    let rest = template
        .strip_prefix("https://")
        .map(|r| ("https://", r))
        .or_else(|| template.strip_prefix("http://").map(|r| ("http://", r)));
    let Some((scheme, rest)) = rest else {
        bail!("http_map_tiles must start with https:// (or http://)");
    };
    for placeholder in ["{z}", "{x}", "{y}"] {
        if !template.contains(placeholder) {
            bail!("http_map_tiles must contain {{z}}, {{x}} and {{y}}");
        }
    }
    // The origin is copied into a response header: allow URL characters only.
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&()*+,=%{}".contains(c);
    if !template.chars().all(allowed) || template.contains("'") {
        bail!("http_map_tiles contains characters that are not allowed in a URL");
    }
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || host.contains(['{', '}']) {
        bail!("http_map_tiles must use a fixed host name");
    }
    Ok(format!("{scheme}{host}"))
}

/// Standard config locations, lowest priority first.
fn search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = [
        "/usr/etc/g86-car-telemetry/gps-to-mqtt.toml",
        "/etc/g86-car-telemetry/gps-to-mqtt.toml",
        "/etc/gps-to-mqtt/settings.toml",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        paths.push(dir.join("settings.toml"));
        paths.push(dir.join("gps-to-mqtt.toml"));
    }
    paths.push("settings.toml".into());
    paths.push("gps-to-mqtt.toml".into());
    paths
}

fn environment(source: Option<config::Map<String, String>>) -> Environment {
    // No key separator: GPS_TO_MQTT_MQTT_HOST must map to the flat key
    // `mqtt_host`, not to a nested `mqtt.host`.
    Environment::with_prefix("GPS_TO_MQTT")
        .prefix_separator("_")
        .try_parsing(true)
        .source(source)
}

fn build(config_path: Option<&str>, env: Option<config::Map<String, String>>) -> Result<AppConfig> {
    let mut builder = Config::builder();
    let loaded_from = match config_path {
        Some(path) => {
            builder = builder.add_source(File::with_name(path).required(true));
            Some(path.to_string())
        }
        None => {
            let mut found = None;
            for path in search_paths().into_iter().filter(|p| p.exists()) {
                found = Some(path.display().to_string());
                builder = builder.add_source(File::from(path).required(false));
            }
            found
        }
    };
    builder = builder.add_source(environment(env));
    let mut cfg: AppConfig = builder
        .build()
        .context("failed to read configuration")?
        .try_deserialize()
        .context("invalid configuration")?;
    cfg.config_path = loaded_from;
    cfg.validate()?;
    Ok(cfg)
}

/// Load `.env`, config files and environment, then validate.
pub fn load_configuration(config_path: Option<&str>) -> Result<AppConfig> {
    dotenvy::dotenv().ok();
    build(config_path, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Option<config::Map<String, String>> {
        Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    fn toml(contents: &str) -> tempfile_path::TempToml {
        tempfile_path::TempToml::new(contents)
    }

    mod tempfile_path {
        pub struct TempToml(pub std::path::PathBuf);
        impl TempToml {
            pub fn new(contents: &str) -> Self {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let path = std::env::temp_dir().join(format!(
                    "gps-to-mqtt-test-{}-{}.toml",
                    std::process::id(),
                    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                std::fs::write(&path, contents).unwrap();
                Self(path)
            }
            pub fn path(&self) -> &str {
                self.0.to_str().unwrap()
            }
        }
        impl Drop for TempToml {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }

    #[test]
    fn canonical_format_keeps_the_legacy_name_and_custom_namespace() {
        for format in ["canonical", "g86"] {
            let file = toml(&format!(
                "mqtt_formats = [\"{format}\"]\nmqtt_base_topic = \"/telemetry/rover/gps\"\ncar = \"legacy-car\""
            ));
            let config = build(Some(file.path()), env(&[])).unwrap();
            assert!(config.wants(MqttFormat::G86));
            assert_eq!(config.base_topic(), "/telemetry/rover/gps");
            assert_eq!(
                serde_json::to_value(config.mqtt_formats).unwrap(),
                serde_json::json!(["canonical"])
            );
        }
    }

    #[test]
    fn defaults_are_plug_and_play() {
        let c = AppConfig::default();
        assert_eq!(c.connection_type, ConnectionType::Serial);
        assert_eq!(c.port_name, "auto");
        assert_eq!(c.baud_rate, 0);
        assert_eq!(c.receiver, ReceiverProfile::Generic);
        assert_eq!(c.mqtt_formats, default_mqtt_formats());
        assert_eq!(c.http_bind, "127.0.0.1");
        assert!(c.validate().is_ok());
    }

    #[test]
    fn multi_word_env_vars_reach_flat_keys() {
        // Regression: a "_" key separator used to turn MQTT_HOST into mqtt.host.
        let f = toml("");
        let c = build(
            Some(f.path()),
            env(&[
                ("GPS_TO_MQTT_MQTT_HOST", "broker.lan"),
                ("GPS_TO_MQTT_MQTT_PORT", "8883"),
                ("GPS_TO_MQTT_CONNECTION_TYPE", "tcp"),
                ("GPS_TO_MQTT_TCP_HOST", "10.0.0.2"),
                ("GPS_TO_MQTT_TCP_PORT", "9001"),
                ("GPS_TO_MQTT_MQTT_FORMATS", "legacy, homeassistant"),
                ("GPS_TO_MQTT_MQTT_PASSWORD", "12345"),
                ("GPS_TO_MQTT_INIT_COMMANDS", "PMTK220,100;PMTK251,115200"),
            ]),
        )
        .unwrap();
        assert_eq!(c.mqtt_host, "broker.lan");
        assert_eq!(c.mqtt_port, 8883);
        assert_eq!(c.connection_type, ConnectionType::Tcp);
        assert_eq!(c.tcp_port, Some(9001));
        assert_eq!(
            c.mqtt_formats,
            [MqttFormat::Legacy, MqttFormat::Homeassistant]
        );
        assert_eq!(c.mqtt_password.as_deref(), Some("12345"));
        assert_eq!(c.init_commands, ["PMTK220,100", "PMTK251,115200"]);
    }

    #[test]
    fn env_overrides_file_and_legacy_keys_are_accepted() {
        let f = toml(
            r#"
            mqtt_host = "file-host"
            baud_rate = 9600
            port_name = "/dev/ttyACM0"
            prometheus_enabled = true
            prometheus_port = 9100
            set_gps_to_10hz = true
            track_mode = "disabled"
            "#,
        );
        let c = build(
            Some(f.path()),
            env(&[("GPS_TO_MQTT_MQTT_HOST", "env-host")]),
        )
        .unwrap();
        assert_eq!(c.mqtt_host, "env-host");
        assert_eq!(c.baud_rate, 9600);
        assert!(c.http_enabled);
        assert_eq!(c.http_port, 9100);
        assert_eq!(c.effective_receiver(), ReceiverProfile::Ublox);
        assert_eq!(c.effective_update_rate(), Some(10));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let cases: Vec<Box<dyn Fn(&mut AppConfig)>> = vec![
            Box::new(|c| c.baud_rate = 12345),
            Box::new(|c| c.port_name.clear()),
            Box::new(|c| c.mqtt_host.clear()),
            Box::new(|c| c.mqtt_port = 0),
            Box::new(|c| c.mqtt_base_topic = Some("/".into())),
            Box::new(|c| c.mqtt_base_topic = Some("a/#".into())),
            Box::new(|c| c.mqtt_qos = 2),
            Box::new(|c| c.mqtt_formats.clear()),
            Box::new(|c| c.mqtt_client_cert_file = Some("c.pem".into())),
            Box::new(|c| c.log_level = "verbose".into()),
            Box::new(|c| c.telemetry_log = Some("relative.ndjson".into())),
            Box::new(|c| c.connection_type = ConnectionType::Tcp),
            Box::new(|c| c.connection_type = ConnectionType::File),
            Box::new(|c| {
                c.connection_type = ConnectionType::Udp;
                c.udp_bind = "nope".into()
            }),
            Box::new(|c| c.update_rate_hz = Some(10)),
            Box::new(|c| {
                c.receiver = ReceiverProfile::Ublox;
                c.update_rate_hz = Some(50)
            }),
            Box::new(|c| {
                c.http_enabled = true;
                c.http_bind = "localhost:80".into()
            }),
            Box::new(|c| c.nmea_tcp_listen = Some("10110".into())),
            Box::new(|c| c.init_commands = vec!["hex:zz".into()]),
            Box::new(|c| c.car = Some("Golf86".into())),
            Box::new(|c| c.car = Some("golf/86".into())),
            Box::new(|c| c.car = Some("-golf".into())),
            Box::new(|c| c.car = Some("a".repeat(33))),
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut c = AppConfig::default();
            mutate(&mut c);
            assert!(c.validate().is_err(), "case {i} should fail");
        }
    }

    #[test]
    fn mqtt_disabled_skips_mqtt_validation() {
        let mut c = AppConfig::default();
        c.mqtt_enabled = false;
        c.mqtt_host.clear();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn unknown_format_is_an_error() {
        let f = toml(r#"mqtt_formats = ["legacy", "kml"]"#);
        assert!(build(Some(f.path()), env(&[])).is_err());
    }

    #[test]
    fn helpers() {
        let mut c = AppConfig::default();
        c.mqtt_base_topic = Some("/CAR/GPS/".into());
        assert_eq!(c.base_topic(), "/CAR/GPS");
        assert_eq!(c.mqtt_address(), "localhost:1883");
        assert_eq!(c.connection_display(), "auto @ auto baud");
        c.connection_type = ConnectionType::Gpsd;
        assert_eq!(c.connection_display(), "gpsd 127.0.0.1:2947");
    }

    #[test]
    fn base_topic_explicit_then_car_then_legacy() {
        // Existing installs (nothing set) keep the legacy topic.
        let f = toml("");
        let c = build(Some(f.path()), env(&[])).unwrap();
        assert_eq!(c.base_topic(), "/GOLF86/GPS");
        assert_eq!(c.base_topic_origin(), "legacy default");
        // `car` selects the G86 namespace, from the file or the environment.
        let f = toml(r#"car = "qa-car""#);
        let c = build(Some(f.path()), env(&[])).unwrap();
        assert_eq!(c.base_topic(), "/g86/qa-car/gps");
        let c = build(Some(f.path()), env(&[("GPS_TO_MQTT_CAR", "golf86")])).unwrap();
        assert_eq!(c.base_topic(), "/g86/golf86/gps");
        let c = build(Some(f.path()), env(&[("GPS_TO_MQTT_CAR", "86")])).unwrap();
        assert_eq!(c.base_topic(), "/g86/86/gps");
        // An explicit base topic always wins.
        let f = toml("car = \"qa-car\"\nmqtt_base_topic = \"/CAR/GPS/\"");
        let c = build(Some(f.path()), env(&[])).unwrap();
        assert_eq!(c.base_topic(), "/CAR/GPS");
        assert_eq!(c.base_topic_origin(), "mqtt_base_topic");
        assert!(build(Some(f.path()), env(&[("GPS_TO_MQTT_CAR", "Bad Car")])).is_err());
    }

    #[test]
    fn secrets_are_redacted() {
        let mut c = AppConfig::default();
        c.mqtt_password = Some("pw".into());
        let v = c.redacted();
        assert_eq!(v["mqtt_password"], "***");
        assert!(v["http_token"].is_null());
        assert_eq!(v["mqtt_host"], "localhost");
    }

    #[test]
    fn map_tile_templates() {
        assert_eq!(
            map_origin(&default_map_tiles()).unwrap(),
            "https://tile.openstreetmap.org"
        );
        assert_eq!(
            map_origin("http://10.0.0.5:8080/osm/{z}/{x}/{y}.png").unwrap(),
            "http://10.0.0.5:8080"
        );
        for bad in [
            "tile.openstreetmap.org/{z}/{x}/{y}.png",
            "https://tile.openstreetmap.org/{z}/{x}.png",
            "https://{s}.tile.example/{z}/{x}/{y}.png",
            "https://evil.example/{z}/{x}/{y}.png; script-src *",
            "https://evil.example/{z}/{x}/{y}.png'",
        ] {
            assert!(map_origin(bad).is_err(), "{bad}");
        }
        let mut c = AppConfig::default();
        c.http_map_tiles.clear();
        assert!(c.validate().is_ok(), "empty disables the map");
    }

    #[test]
    fn password_file_is_trimmed() {
        let f = toml("s3cret\n");
        let mut c = AppConfig::default();
        c.mqtt_password = Some("ignored".into());
        c.mqtt_password_file = Some(f.path().into());
        assert_eq!(c.mqtt_password().unwrap().as_deref(), Some("s3cret"));
    }
}
