//! Bridge health for the G86 car computer: `<base>/health` (retained JSON,
//! `contracts/CAR_COMPUTER.md` §2). Published every 5 s and at once when the
//! state or the receiver link changes.
//!
//! * `down`: the receiver link is not connected, or no NMEA sentence for 10 s.
//! * `degraded`: link up but no valid fix / last position older than 2 s, or an
//!   error counter rose within the last 5 s.
//! * `ok`: otherwise.

use crate::shared::LinkStatus;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

const RATE_WINDOW: Duration = Duration::from_secs(5);
const FRESH: Duration = Duration::from_secs(2);
const SILENT: Duration = Duration::from_secs(10);
const ERROR_WINDOW: Duration = Duration::from_secs(5);
pub const INTERVAL: Duration = Duration::from_secs(5);

/// What the tracker needs from the rest of the process at one instant.
pub struct Inputs {
    pub link: LinkStatus,
    pub last_data_at: Option<Instant>,
    pub fix_valid: bool,
    pub errors: BTreeMap<&'static str, u64>,
    pub clock_synced: bool,
}

#[derive(Default)]
pub struct HealthTracker {
    published: VecDeque<Instant>,
    channels: HashMap<String, Instant>,
    last_position: Option<Instant>,
    error_total: Option<u64>,
    error_rose_at: Option<Instant>,
    last_sent: Option<(Instant, &'static str, &'static str)>,
}

fn link_name(link: LinkStatus) -> &'static str {
    match link {
        LinkStatus::Connected => "connected",
        LinkStatus::Connecting => "connecting",
        LinkStatus::Disconnected | LinkStatus::Disabled => "disconnected",
    }
}

impl HealthTracker {
    /// A position packet was published.
    pub fn on_position(&mut self, now: Instant, packet: &str) {
        self.published.push_back(now);
        self.last_position = Some(now);
        if let Ok(Value::Object(p)) = serde_json::from_str::<Value>(packet)
            && let Some(Value::Object(channels)) = p.get("channels")
        {
            for key in channels.keys() {
                match self.channels.get_mut(key.as_str()) {
                    Some(t) => *t = now,
                    None => {
                        // Bounded: the encoder emits a fixed handful of keys.
                        if self.channels.len() < 64 {
                            self.channels.insert(key.clone(), now);
                        }
                    }
                }
            }
        }
    }

    /// Current document, its state and link.
    pub fn document(
        &mut self,
        now: Instant,
        wall_ms: i64,
        inputs: &Inputs,
    ) -> (Value, &'static str, &'static str) {
        while self
            .published
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > RATE_WINDOW)
        {
            self.published.pop_front();
        }
        let total: u64 = inputs.errors.values().sum();
        if self.error_total.is_some_and(|before| total > before) {
            self.error_rose_at = Some(now);
        }
        self.error_total = Some(total);

        let link = link_name(inputs.link);
        let age = self.last_position.map(|t| now.saturating_duration_since(t));
        let silent = inputs
            .last_data_at
            .is_none_or(|t| now.saturating_duration_since(t) > SILENT);
        let errors_rising = self
            .error_rose_at
            .is_some_and(|t| now.saturating_duration_since(t) <= ERROR_WINDOW);
        let state = if inputs.link != LinkStatus::Connected || silent {
            "down"
        } else if !inputs.fix_valid || age.is_none_or(|a| a > FRESH) || errors_rising {
            "degraded"
        } else {
            "ok"
        };
        let mut fresh: Vec<&str> = self
            .channels
            .iter()
            .filter(|(_, t)| now.saturating_duration_since(**t) <= FRESH)
            .map(|(k, _)| k.as_str())
            .collect();
        fresh.sort_unstable();
        let rate = self.published.len() as f64 / RATE_WINDOW.as_secs_f64();
        let doc = json!({
            "schema": 1,
            "bridge": "gps-to-mqtt",
            "version": env!("CARGO_PKG_VERSION"),
            "bootId": crate::position::boot_id(),
            "timestampMs": wall_ms,
            "state": state,
            "link": link,
            "sources": ["gps-to-mqtt"],
            "rateHz": (rate * 10.0).round() / 10.0,
            "ageMs": age.map(|a| a.as_millis() as u64),
            "channels": fresh,
            "errors": inputs.errors,
            "clockSynced": inputs.clock_synced,
            "fix": inputs.fix_valid,
        });
        (doc, state, link)
    }

    /// Publish now: every [`INTERVAL`], or when the state or link changed.
    pub fn due(&self, now: Instant, state: &str, link: &str) -> bool {
        match self.last_sent {
            None => true,
            Some((at, s, l)) => {
                s != state || l != link || now.saturating_duration_since(at) >= INTERVAL
            }
        }
    }

    pub fn sent(&mut self, now: Instant, state: &'static str, link: &'static str) {
        self.last_sent = Some((now, state, link));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &str =
        r#"{"schema":1,"source":"nmea","channels":{"hdop":0.8,"satellitesUsed":12}}"#;

    fn inputs(link: LinkStatus, data: Option<Instant>, fix: bool, checksum: u64) -> Inputs {
        Inputs {
            link,
            last_data_at: data,
            fix_valid: fix,
            errors: BTreeMap::from([("checksum", checksum), ("reconnects", 0)]),
            clock_synced: true,
        }
    }

    #[test]
    fn states_follow_link_data_fix_and_errors() {
        let t0 = Instant::now();
        let mut h = HealthTracker::default();
        let (doc, state, link) = h.document(t0, 1, &inputs(LinkStatus::Connecting, None, false, 0));
        assert_eq!((state, link), ("down", "connecting"));
        assert!(doc["ageMs"].is_null());
        assert_eq!(doc["rateHz"], 0.0);

        // Link up, sentences, no fix yet → degraded.
        let (_, state, _) = h.document(t0, 1, &inputs(LinkStatus::Connected, Some(t0), false, 0));
        assert_eq!(state, "degraded");

        // 10 Hz positions for 5 s → ok at 10 Hz with fresh channels.
        for i in 0..50 {
            h.on_position(t0 + Duration::from_millis(100 * i), PACKET);
        }
        let t1 = t0 + Duration::from_millis(4950);
        let (doc, state, link) =
            h.document(t1, 2, &inputs(LinkStatus::Connected, Some(t1), true, 0));
        assert_eq!((state, link), ("ok", "connected"));
        assert_eq!(doc["rateHz"], 10.0);
        assert_eq!(doc["ageMs"], 50);
        assert_eq!(doc["channels"], json!(["hdop", "satellitesUsed"]));
        assert_eq!(doc["errors"]["checksum"], 0);

        // A checksum error → degraded for 5 s, then ok again.
        let t2 = t1 + Duration::from_millis(50);
        h.on_position(t2, PACKET);
        let (_, state, _) = h.document(t2, 3, &inputs(LinkStatus::Connected, Some(t2), true, 1));
        assert_eq!(state, "degraded");
        let mut t = t2;
        for _ in 0..6 {
            t += Duration::from_secs(1);
            h.on_position(t, PACKET);
        }
        let (_, state, _) = h.document(t, 4, &inputs(LinkStatus::Connected, Some(t), true, 1));
        assert_eq!(state, "ok");

        // Positions stop (fix lost) → degraded; data stops for > 10 s → down.
        let t3 = t + Duration::from_secs(3);
        let (doc, state, _) = h.document(t3, 5, &inputs(LinkStatus::Connected, Some(t3), true, 1));
        assert_eq!(state, "degraded");
        assert_eq!(doc["channels"], json!([]));
        let t4 = t3 + Duration::from_secs(11);
        let (_, state, _) = h.document(t4, 6, &inputs(LinkStatus::Connected, Some(t3), true, 1));
        assert_eq!(state, "down");
    }

    #[test]
    fn document_matches_the_contract_shape() {
        let t0 = Instant::now();
        let mut h = HealthTracker::default();
        h.on_position(t0, PACKET);
        let (doc, _, _) = h.document(
            t0,
            1_792_664_130_000,
            &inputs(LinkStatus::Connected, Some(t0), true, 0),
        );
        for key in [
            "schema",
            "bridge",
            "version",
            "bootId",
            "timestampMs",
            "state",
            "link",
            "sources",
            "rateHz",
            "ageMs",
            "channels",
            "errors",
            "clockSynced",
        ] {
            assert!(doc.get(key).is_some(), "missing {key}");
        }
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["bridge"], "gps-to-mqtt");
        assert_eq!(doc["sources"], json!(["gps-to-mqtt"]));
        assert_eq!(doc["timestampMs"], 1_792_664_130_000_i64);
        assert!(doc["clockSynced"].is_boolean());
    }

    #[test]
    fn published_every_interval_and_on_change() {
        let t0 = Instant::now();
        let mut h = HealthTracker::default();
        assert!(h.due(t0, "ok", "connected"));
        h.sent(t0, "ok", "connected");
        assert!(!h.due(t0 + Duration::from_secs(1), "ok", "connected"));
        assert!(h.due(t0 + Duration::from_secs(1), "degraded", "connected"));
        assert!(h.due(t0 + Duration::from_secs(1), "ok", "connecting"));
        assert!(h.due(t0 + INTERVAL, "ok", "connected"));
    }
}
