//! Process-wide state shared by the hub, outputs, HTTP API and TUI.
//!
//! * [`Bus`] — lossy broadcast of live events. A slow consumer (MQTT during an
//!   outage, a stalled TCP client) lags and skips; it never back-pressures
//!   acquisition.
//! * [`Snapshot`] — latest values for request/response readers (HTTP, TUI).
//!   Guarded by a std `RwLock` that is never held across `.await`.
//! * [`Counters`] — lock-free statistics exported to Prometheus.

use crate::models::{Fix, Sky};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;
use tokio::sync::broadcast;

pub const RAW_HISTORY: usize = 500;
pub const MESSAGE_HISTORY: usize = 100;
const BUS_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
pub enum BusEvent {
    /// Verified NMEA sentence (no line terminator).
    Nmea(Arc<str>),
    Fix(Arc<Fix>),
    /// Sky view, at most once per second.
    Sky(Arc<Sky>),
    /// G86 external-GPS packet (JSON).
    G86(Arc<str>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    Disabled,
    Connecting,
    Connected,
    Disconnected,
}

impl LinkStatus {
    pub fn is_up(self) -> bool {
        self == Self::Connected
    }
}

#[derive(Default)]
pub struct Counters {
    pub bytes: AtomicU64,
    pub sentences: AtomicU64,
    pub checksum_errors: AtomicU64,
    pub malformed: AtomicU64,
    pub fixes: AtomicU64,
    pub mqtt_published: AtomicU64,
    pub mqtt_dropped: AtomicU64,
    pub archive_errors: AtomicU64,
    pub clients: AtomicU64,
    /// Receiver link losses (connected → anything else).
    pub reconnects: AtomicU64,
}

impl Counters {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// Static description of how the process is wired.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Info {
    pub version: &'static str,
    pub source: String,
    pub receiver: String,
    pub mqtt: Option<String>,
    pub outputs: Vec<String>,
}

pub struct Snapshot {
    pub fix: Option<Arc<Fix>>,
    pub last_fix_at: Option<Instant>,
    pub sky: Arc<Sky>,
    pub raw: VecDeque<Arc<str>>,
    pub messages: VecDeque<String>,
    pub source: LinkStatus,
    /// Resolved device details, e.g. `/dev/ttyACM0 @ 9600 (u-blox 7)`.
    pub source_detail: String,
    pub source_since: Instant,
    pub last_data_at: Option<Instant>,
    pub mqtt: LinkStatus,
    /// Sentences and fixes per second over the last second.
    pub sentence_rate: f64,
    pub fix_rate: f64,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            fix: None,
            last_fix_at: None,
            sky: Arc::new(Sky::default()),
            raw: VecDeque::with_capacity(RAW_HISTORY),
            messages: VecDeque::with_capacity(MESSAGE_HISTORY),
            source: LinkStatus::Connecting,
            source_detail: String::new(),
            source_since: Instant::now(),
            last_data_at: None,
            mqtt: LinkStatus::Disabled,
            sentence_rate: 0.0,
            fix_rate: 0.0,
        }
    }
}

impl Snapshot {
    pub fn push_raw(&mut self, line: Arc<str>) {
        if self.raw.len() == RAW_HISTORY {
            self.raw.pop_front();
        }
        self.raw.push_back(line);
    }

    pub fn push_message(&mut self, msg: String) {
        if self.messages.len() == MESSAGE_HISTORY {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }

    /// Age of the latest fix in seconds.
    pub fn fix_age_s(&self, now: Instant) -> Option<f64> {
        self.last_fix_at
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
    }
}

pub struct Shared {
    pub info: Info,
    pub counters: Counters,
    pub bus: broadcast::Sender<BusEvent>,
    snapshot: RwLock<Snapshot>,
    pub started: Instant,
}

impl Shared {
    pub fn new(info: Info, mqtt_enabled: bool) -> Arc<Self> {
        let (bus, _) = broadcast::channel(BUS_CAPACITY);
        let snapshot = Snapshot {
            mqtt: if mqtt_enabled {
                LinkStatus::Connecting
            } else {
                LinkStatus::Disabled
            },
            ..Snapshot::default()
        };
        Arc::new(Self {
            info,
            counters: Counters::default(),
            bus,
            snapshot: RwLock::new(snapshot),
            started: Instant::now(),
        })
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Snapshot> {
        self.snapshot.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Snapshot> {
        self.snapshot.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Broadcast without caring whether anyone listens.
    pub fn publish(&self, event: BusEvent) {
        let _ = self.bus.send(event);
    }

    pub fn set_source(&self, status: LinkStatus, detail: impl Into<String>) {
        let detail = detail.into();
        let mut s = self.write();
        if s.source == LinkStatus::Connected && status != LinkStatus::Connected {
            Counters::inc(&self.counters.reconnects);
        }
        if s.source != status {
            s.source = status;
            s.source_since = Instant::now();
        }
        if !detail.is_empty() {
            s.source_detail = detail;
        }
    }

    pub fn set_mqtt(&self, status: LinkStatus) {
        self.write().mqtt = status;
    }

    /// Healthy = source connected and a valid fix within `max_age_s`.
    pub fn health(&self, max_age_s: f64) -> Health {
        let now = Instant::now();
        let s = self.read();
        let fix_age = s.fix_age_s(now);
        let has_fix = s.fix.as_ref().is_some_and(|f| f.valid);
        let fresh = fix_age.is_some_and(|a| a <= max_age_s);
        let status = match (s.source.is_up(), has_fix && fresh) {
            (true, true) => "ok",
            (true, false) => "no_fix",
            (false, _) => "source_down",
        };
        Health {
            status,
            source: s.source,
            source_detail: s.source_detail.clone(),
            mqtt: s.mqtt,
            fix_valid: has_fix,
            fix_age_s: fix_age,
            uptime_s: now.duration_since(self.started).as_secs(),
            version: self.info.version,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub source: LinkStatus,
    pub source_detail: String,
    pub mqtt: LinkStatus,
    pub fix_valid: bool,
    pub fix_age_s: Option<f64>,
    pub uptime_s: u64,
    pub version: &'static str,
}

impl Health {
    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_are_bounded() {
        let mut s = Snapshot::default();
        for i in 0..RAW_HISTORY + 10 {
            s.push_raw(Arc::from(i.to_string()));
            s.push_message(i.to_string());
        }
        assert_eq!(s.raw.len(), RAW_HISTORY);
        assert_eq!(s.messages.len(), MESSAGE_HISTORY);
        assert_eq!(&*s.raw[0], "10");
    }

    #[test]
    fn health_reflects_source_and_fix_freshness() {
        let shared = Shared::new(Info::default(), false);
        assert_eq!(shared.health(5.0).status, "source_down");
        shared.set_source(LinkStatus::Connected, "/dev/ttyACM0");
        assert_eq!(shared.health(5.0).status, "no_fix");
        {
            let mut s = shared.write();
            s.fix = Some(Arc::new(Fix {
                valid: true,
                ..Fix::default()
            }));
            s.last_fix_at = Some(Instant::now());
        }
        let h = shared.health(5.0);
        assert!(h.is_ok());
        assert_eq!(h.source_detail, "/dev/ttyACM0");
        shared.set_source(LinkStatus::Disconnected, "");
        assert_eq!(shared.health(5.0).status, "source_down");
        assert_eq!(shared.health(5.0).mqtt, LinkStatus::Disabled);
    }
}
