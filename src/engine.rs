//! Epoch-based NMEA aggregation.
//!
//! A receiver emits a burst of sentences per navigation epoch (RMC, GGA, GSA,
//! GSV, …). [`Engine`] groups timed sentences by their UTC time and emits one
//! coherent [`Fix`] per epoch:
//!
//! * as soon as both RMC and GGA/GNS for the epoch have been seen (lowest latency), or
//! * when the next epoch starts / the stream goes idle (receivers that emit only
//!   one of them).
//!
//! Untimed sentences (GSA, VTG, HDT, ROT, PUBX) update short-lived auxiliary
//! values that are attached to the next emitted fix and expire when stale, so a
//! receiver that stops sending GSA never leaves an old DOP on new fixes.

use crate::models::{
    AccuracySource, Constellation, Fix, FixMode, FixQuality, KNOTS_TO_MPS, Satellite, Sky,
};
use crate::nmea::Sentence;
use crate::nmea::fields::{int, num, parse_coordinate, parse_date, parse_time};
use crate::telemetry::TelemetryCalculator;
use chrono::{NaiveDate, NaiveTime, Utc};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Auxiliary values older than this are not attached to new fixes.
const AUX_MAX_AGE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy)]
struct Fresh<T> {
    value: T,
    at: Instant,
}

impl<T: Copy> Fresh<T> {
    fn get(slot: &Option<Self>, now: Instant) -> Option<T> {
        slot.filter(|f| now.duration_since(f.at) <= AUX_MAX_AGE)
            .map(|f| f.value)
    }
}

/// Three optional values: (PDOP, HDOP, VDOP) or GST (lat, lon, alt) σ.
type Triple = (Option<f64>, Option<f64>, Option<f64>);

#[derive(Default)]
struct Aux {
    mode: Option<Fresh<FixMode>>,
    /// (PDOP, HDOP, VDOP) — GSA field order.
    dop: Option<Fresh<Triple>>,
    gst: Option<Fresh<Triple>>,
    receiver_accuracy: Option<Fresh<f64>>,
    heading: Option<Fresh<f64>>,
    rot_dps: Option<Fresh<f64>>,
    speed_course: Option<Fresh<(Option<f64>, Option<f64>)>>,
}

#[derive(Default)]
struct Epoch {
    time: Option<NaiveTime>,
    fix: Fix,
    status_void: bool,
    quality: Option<FixQuality>,
    have_rmc: bool,
    have_gga: bool,
    emitted: bool,
    touched: bool,
}

struct SatEntry {
    sat: Satellite,
    seen: Instant,
}

/// Result of feeding one sentence.
#[derive(Default)]
pub struct Ingest {
    pub fix: Option<Fix>,
    /// Receiver text message (TXT).
    pub text: Option<String>,
}

pub struct Engine {
    date: Option<NaiveDate>,
    epoch: Epoch,
    aux: Aux,
    sats: HashMap<(Constellation, u32), SatEntry>,
    used: HashSet<(Option<Constellation>, u32)>,
    used_epoch: Option<Option<NaiveTime>>,
    satellite_timeout: Duration,
    sequence: u64,
    telemetry: Option<TelemetryCalculator>,
    hold: Option<crate::hold::StaticHold>,
    last_input: Option<Instant>,
}

impl Engine {
    pub fn new(telemetry_window: Option<usize>, satellite_timeout: Duration) -> Self {
        Self {
            date: None,
            epoch: Epoch::default(),
            aux: Aux::default(),
            sats: HashMap::new(),
            used: HashSet::new(),
            used_epoch: None,
            satellite_timeout,
            sequence: 0,
            telemetry: telemetry_window.map(TelemetryCalculator::new),
            hold: None,
            last_input: None,
        }
    }

    /// Enable static hold for stationary positions.
    pub fn with_hold(mut self, hold: Option<crate::hold::HoldConfig>) -> Self {
        self.hold = hold.map(crate::hold::StaticHold::new);
        self
    }

    /// Feed one verified sentence.
    pub fn ingest(&mut self, s: &Sentence<'_>, now: Instant) -> Ingest {
        self.last_input = Some(now);
        let mut out = Ingest::default();
        match s.kind {
            "RMC" => out.fix = self.rmc(s, now),
            "GGA" => out.fix = self.gga(s, now),
            "GNS" => out.fix = self.gns(s, now),
            "GLL" => out.fix = self.gll(s, now),
            "VTG" => self.vtg(s, now),
            "GSA" => self.gsa(s, now),
            "GSV" => self.gsv(s, now),
            "GST" => self.gst(s, now),
            "ZDA" => self.zda(s),
            "HDT" | "THS" => self.heading(s, now),
            "ROT" => self.rot(s, now),
            "TXT" => out.text = Some(s.field(3).trim().to_string()).filter(|t| !t.is_empty()),
            "UBX" if s.field(0) == "00" => self.pubx_position(s, now),
            _ => {}
        }
        out
    }

    /// Emit the pending epoch if the stream has been idle for `idle`.
    pub fn flush_if_idle(&mut self, now: Instant, idle: Duration) -> Option<Fix> {
        let idle_long_enough = self
            .last_input
            .is_some_and(|t| now.duration_since(t) >= idle);
        if idle_long_enough && self.epoch.touched && !self.epoch.emitted {
            self.epoch.emitted = true;
            return Some(self.finish(now));
        }
        None
    }

    /// Current sky view; satellites not reported within the timeout are dropped.
    pub fn sky(&mut self, now: Instant) -> Sky {
        let timeout = self.satellite_timeout;
        self.sats
            .retain(|_, e| now.duration_since(e.seen) <= timeout);
        let mut satellites: Vec<Satellite> = self
            .sats
            .values()
            .map(|e| {
                let mut sat = e.sat.clone();
                sat.used = self.used.contains(&(Some(sat.constellation), sat.prn))
                    || self.used.contains(&(None, sat.prn));
                sat
            })
            .collect();
        satellites.sort_by_key(|s| (s.constellation, s.prn));
        let (pdop, hdop, vdop) = Fresh::get(&self.aux.dop, now).unwrap_or_default();
        Sky {
            in_view: satellites.len(),
            used: satellites.iter().filter(|s| s.used).count(),
            satellites,
            hdop,
            vdop,
            pdop,
        }
    }

    // ── epoch handling ───────────────────────────────────────────────────

    /// Switch to the epoch of `time`, returning the previous epoch's fix if it
    /// was never emitted.
    fn enter_epoch(&mut self, time: Option<NaiveTime>, now: Instant) -> Option<Fix> {
        let time = time?;
        if self.epoch.time == Some(time) {
            return None;
        }
        let pending = (self.epoch.touched && !self.epoch.emitted).then(|| self.finish(now));
        self.epoch = Epoch {
            time: Some(time),
            ..Epoch::default()
        };
        pending
    }

    fn complete_if_ready(&mut self, now: Instant) -> Option<Fix> {
        if self.epoch.have_rmc && self.epoch.have_gga && !self.epoch.emitted {
            self.epoch.emitted = true;
            return Some(self.finish(now));
        }
        None
    }

    fn finish(&mut self, now: Instant) -> Fix {
        // Judge auxiliary freshness against the epoch's own input time, not
        // the moment an idle flush happens.
        let now = self.last_input.map_or(now, |t| t.min(now));
        let mut fix = self.epoch.fix.clone();
        fix.utc_time = self.epoch.time;
        fix.utc_date = self.date;
        fix.time = match (self.date, self.epoch.time) {
            (Some(d), Some(t)) => Some(d.and_time(t).and_utc()),
            _ => None,
        };
        fix.received_ms = Utc::now().timestamp_millis();

        let quality = self.epoch.quality.unwrap_or(if self.epoch.status_void {
            FixQuality::Invalid
        } else if fix.latitude.is_some() {
            FixQuality::Gps
        } else {
            FixQuality::Invalid
        });
        fix.quality = quality;
        fix.valid = !self.epoch.status_void
            && quality != FixQuality::Invalid
            && fix.latitude.is_some()
            && fix.longitude.is_some();

        fix.mode = Fresh::get(&self.aux.mode, now).unwrap_or(match (fix.valid, fix.altitude_m) {
            (false, _) => FixMode::NoFix,
            (true, Some(_)) => FixMode::Fix3D,
            (true, None) => FixMode::Fix2D,
        });
        if !fix.valid {
            fix.mode = FixMode::NoFix;
        }

        if let Some((pdop, hdop, vdop)) = Fresh::get(&self.aux.dop, now) {
            fix.pdop = pdop;
            fix.hdop = fix.hdop.or(hdop);
            fix.vdop = vdop;
        }
        if (fix.speed_mps.is_none() || fix.course_deg.is_none())
            && let Some((speed, course)) = Fresh::get(&self.aux.speed_course, now)
        {
            fix.speed_mps = fix.speed_mps.or(speed);
            fix.course_deg = fix.course_deg.or(course);
        }
        fix.heading_deg = Fresh::get(&self.aux.heading, now);
        fix.rate_of_turn_dps = Fresh::get(&self.aux.rot_dps, now);

        if let Some((lat, lon, alt)) = Fresh::get(&self.aux.gst, now) {
            fix.error_lat_m = lat;
            fix.error_lon_m = lon;
            fix.error_alt_m = alt;
        }
        (fix.accuracy_m, fix.accuracy_source) = match (
            fix.error_lat_m.zip(fix.error_lon_m),
            Fresh::get(&self.aux.receiver_accuracy, now),
            fix.hdop,
        ) {
            (Some((la, lo)), _, _) => (Some((la * la + lo * lo).sqrt()), Some(AccuracySource::Gst)),
            (None, Some(acc), _) => (Some(acc), Some(AccuracySource::Receiver)),
            (None, None, Some(h)) => (Some((h * 5.0).max(3.0)), Some(AccuracySource::HdopEstimate)),
            _ => (None, None),
        };

        if fix.satellites_used.is_none() && !self.used.is_empty() {
            fix.satellites_used = Some(self.used.len() as u32);
        }
        let in_view = self
            .sats
            .values()
            .filter(|e| now.duration_since(e.seen) <= self.satellite_timeout)
            .count();
        fix.satellites_in_view = (in_view > 0).then_some(in_view as u32);

        if let Some(hold) = self.hold.as_mut() {
            hold.apply(&mut fix);
        }
        if let Some((lat, lon)) = fix.position() {
            fix.mgrs = crate::mgrs::mgrs(lat, lon, 10);
            fix.mgrs_8 = fix.mgrs.as_deref().and_then(crate::mgrs::to_8_digit);
        }

        fix.sequence = self.sequence;
        self.sequence += 1;

        if let Some(calc) = self.telemetry.as_mut() {
            fix.telemetry = Some(calc.update(&fix));
        }
        fix
    }

    fn set_position(&mut self, lat: &str, ns: &str, lon: &str, ew: &str) {
        if let (Ok(lat), Ok(lon)) = (parse_coordinate(lat, ns), parse_coordinate(lon, ew)) {
            self.epoch.fix.latitude = Some(lat);
            self.epoch.fix.longitude = Some(lon);
        }
    }

    // ── sentence handlers ────────────────────────────────────────────────

    fn rmc(&mut self, s: &Sentence<'_>, now: Instant) -> Option<Fix> {
        let pending = self.enter_epoch(parse_time(s.field(0)).ok(), now);
        if let Ok(date) = parse_date(s.field(8)) {
            self.date = Some(date);
        }
        let e = &mut self.epoch;
        e.touched = true;
        e.have_rmc = true;
        e.status_void |= s.field(1) != "A";
        if let Some(q) = s
            .field(11)
            .chars()
            .next()
            .and_then(FixQuality::from_mode_char)
        {
            e.quality.get_or_insert(q);
        }
        if let Some(knots) = num(s.field(6)) {
            e.fix.speed_mps = Some(knots * KNOTS_TO_MPS);
        }
        e.fix.course_deg = num(s.field(7)).filter(|c| (0.0..=360.0).contains(c));
        e.fix.magnetic_variation_deg =
            num(s.field(9)).map(|v| if s.field(10) == "W" { -v } else { v });
        self.set_position(s.field(2), s.field(3), s.field(4), s.field(5));
        pending.or_else(|| self.complete_if_ready(now))
    }

    fn gga(&mut self, s: &Sentence<'_>, now: Instant) -> Option<Fix> {
        let pending = self.enter_epoch(parse_time(s.field(0)).ok(), now);
        let e = &mut self.epoch;
        e.touched = true;
        e.have_gga = true;
        // GGA quality is authoritative over the RMC mode character.
        e.quality = Some(FixQuality::from_gga(s.field(5)));
        e.fix.satellites_used = int(s.field(6));
        e.fix.hdop = num(s.field(7));
        e.fix.altitude_m = num(s.field(8));
        e.fix.geoid_separation_m = num(s.field(10));
        e.fix.dgps_age_s = num(s.field(12));
        e.fix.dgps_station = int(s.field(13));
        self.set_position(s.field(1), s.field(2), s.field(3), s.field(4));
        pending.or_else(|| self.complete_if_ready(now))
    }

    fn gns(&mut self, s: &Sentence<'_>, now: Instant) -> Option<Fix> {
        let pending = self.enter_epoch(parse_time(s.field(0)).ok(), now);
        let e = &mut self.epoch;
        e.touched = true;
        e.have_gga = true;
        // Mode string has one character per constellation; take the best one.
        let quality = s
            .field(5)
            .chars()
            .filter_map(FixQuality::from_mode_char)
            .max_by_key(|q| match q {
                FixQuality::Rtk => 6,
                FixQuality::FloatRtk => 5,
                FixQuality::Dgps => 4,
                FixQuality::Pps => 3,
                FixQuality::Gps => 2,
                FixQuality::Estimated | FixQuality::Manual | FixQuality::Simulation => 1,
                FixQuality::Invalid => 0,
            });
        if let Some(q) = quality {
            e.quality.get_or_insert(q);
        }
        e.fix.satellites_used = e.fix.satellites_used.or(int(s.field(6)));
        e.fix.hdop = e.fix.hdop.or(num(s.field(7)));
        e.fix.altitude_m = e.fix.altitude_m.or(num(s.field(8)));
        e.fix.geoid_separation_m = e.fix.geoid_separation_m.or(num(s.field(9)));
        self.set_position(s.field(1), s.field(2), s.field(3), s.field(4));
        pending.or_else(|| self.complete_if_ready(now))
    }

    fn gll(&mut self, s: &Sentence<'_>, now: Instant) -> Option<Fix> {
        let pending = self.enter_epoch(parse_time(s.field(4)).ok(), now);
        self.epoch.touched = true;
        self.epoch.status_void |= s.field(5) != "A";
        if self.epoch.fix.latitude.is_none() {
            self.set_position(s.field(0), s.field(1), s.field(2), s.field(3));
        }
        pending
    }

    fn vtg(&mut self, s: &Sentence<'_>, now: Instant) {
        // NMEA 2.3+: "054.7,T,034.4,M,005.5,N,010.2,K,A"; legacy: "054.7,034.4,005.5,010.2".
        let (course, knots, kph) = if s.field(1) == "T" {
            (num(s.field(0)), num(s.field(4)), num(s.field(6)))
        } else {
            (num(s.field(0)), num(s.field(2)), num(s.field(3)))
        };
        let speed = kph.map(|k| k / 3.6).or(knots.map(|k| k * KNOTS_TO_MPS));
        self.aux.speed_course = Some(Fresh {
            value: (speed, course),
            at: now,
        });
    }

    fn gsa(&mut self, s: &Sentence<'_>, now: Instant) {
        let mode = match s.field(1) {
            "2" => FixMode::Fix2D,
            "3" => FixMode::Fix3D,
            _ => FixMode::NoFix,
        };
        self.aux.mode = Some(Fresh {
            value: mode,
            at: now,
        });
        self.aux.dop = Some(Fresh {
            value: (num(s.field(14)), num(s.field(15)), num(s.field(16))),
            at: now,
        });
        // Several GSA sentences (one per constellation) share an epoch.
        if self.used_epoch != Some(self.epoch.time) {
            self.used.clear();
            self.used_epoch = Some(self.epoch.time);
        }
        let system = Constellation::from_system_id(s.field(17)).or(match s.talker {
            "GN" | "GP" => None,
            t => Some(Constellation::classify(t, 0)),
        });
        for i in 2..14 {
            if let Some(prn) = int(s.field(i)) {
                self.used.insert((system, prn));
            }
        }
    }

    fn gsv(&mut self, s: &Sentence<'_>, now: Instant) {
        let groups = s.len().saturating_sub(3) / 4;
        for g in 0..groups {
            let base = 3 + g * 4;
            let Some(prn) = int(s.field(base)) else {
                continue;
            };
            let constellation = Constellation::classify(s.talker, prn);
            let snr = s.field(base + 3).parse::<i32>().ok();
            let entry = self
                .sats
                .entry((constellation, prn))
                .or_insert_with(|| SatEntry {
                    sat: Satellite {
                        constellation,
                        prn,
                        elevation_deg: None,
                        azimuth_deg: None,
                        snr_dbhz: None,
                        used: false,
                    },
                    seen: now,
                });
            entry.sat.elevation_deg = s.field(base + 1).parse().ok();
            entry.sat.azimuth_deg = s.field(base + 2).parse().ok();
            // NMEA 4.10 reports each signal (L1, L5 …) separately: keep the
            // strongest one reported within this cycle.
            entry.sat.snr_dbhz = match (snr, entry.sat.snr_dbhz) {
                (Some(new), Some(old))
                    if now.duration_since(entry.seen) < Duration::from_millis(500) =>
                {
                    Some(new.max(old))
                }
                (new, _) => new,
            };
            entry.seen = now;
        }
    }

    fn gst(&mut self, s: &Sentence<'_>, now: Instant) {
        let positive = |v: Option<f64>| v.filter(|v| *v > 0.0);
        let value = (
            positive(num(s.field(5))),
            positive(num(s.field(6))),
            positive(num(s.field(7))),
        );
        if value != (None, None, None) {
            self.aux.gst = Some(Fresh { value, at: now });
        }
    }

    fn zda(&mut self, s: &Sentence<'_>) {
        if let (Some(d), Some(m), Some(y)) = (int(s.field(1)), int(s.field(2)), int(s.field(3)))
            && let Some(date) = NaiveDate::from_ymd_opt(y as i32, m, d)
        {
            self.date = Some(date);
        }
    }

    fn heading(&mut self, s: &Sentence<'_>, now: Instant) {
        let valid = s.kind == "HDT" || s.field(1) == "A";
        if let Some(h) = num(s.field(0)).filter(|h| valid && (0.0..=360.0).contains(h)) {
            self.aux.heading = Some(Fresh {
                value: h % 360.0,
                at: now,
            });
        }
    }

    fn rot(&mut self, s: &Sentence<'_>, now: Instant) {
        if s.field(1) == "A"
            && let Some(deg_per_min) = num(s.field(0))
        {
            self.aux.rot_dps = Some(Fresh {
                value: deg_per_min / 60.0,
                at: now,
            });
        }
    }

    /// u-blox PUBX,00 carries receiver-estimated horizontal accuracy.
    fn pubx_position(&mut self, s: &Sentence<'_>, now: Instant) {
        if let Some(h_acc) = num(s.field(8)).filter(|v| *v > 0.0) {
            self.aux.receiver_accuracy = Some(Fresh {
                value: h_acc,
                at: now,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmea::with_checksum;

    fn feed(engine: &mut Engine, body: &str, now: Instant) -> Ingest {
        let line = with_checksum(body);
        let s = Sentence::parse(&line, true).unwrap();
        engine.ingest(&s, now)
    }

    fn engine() -> Engine {
        Engine::new(Some(3), Duration::from_secs(10))
    }

    const RMC: &str = "GNRMC,120000.00,A,5658.0000,N,02414.0000,E,50.0,85.0,230926,,,A";
    const GGA: &str = "GNGGA,120000.00,5658.0000,N,02414.0000,E,1,12,0.8,12.0,M,22.1,M,,";

    #[test]
    fn emits_one_fix_when_rmc_and_gga_complete() {
        let mut e = engine();
        let t = Instant::now();
        assert!(feed(&mut e, RMC, t).fix.is_none());
        let fix = feed(&mut e, GGA, t).fix.expect("fix after RMC+GGA");
        assert!(fix.valid);
        assert_eq!(fix.quality, FixQuality::Gps);
        assert_eq!(fix.satellites_used, Some(12));
        assert_eq!(fix.altitude_m, Some(12.0));
        assert!((fix.speed_mps.unwrap() - 50.0 * KNOTS_TO_MPS).abs() < 1e-9);
        assert_eq!(fix.time.unwrap().timestamp_millis(), 1_790_164_800_000);
        assert_eq!(fix.accuracy_source, Some(AccuracySource::HdopEstimate));
        assert_eq!(fix.accuracy_m, Some(4.0));
        assert_eq!(
            fix.mgrs,
            crate::mgrs::mgrs(fix.latitude.unwrap(), fix.longitude.unwrap(), 10)
        );
        assert!(fix.mgrs.as_deref().unwrap().starts_with("35V "));
        assert_eq!(
            fix.mgrs_8.as_deref().map(str::len),
            Some("35V LC 1234 6789".len())
        );
        // Duplicate sentences in the same epoch do not emit again.
        assert!(feed(&mut e, GGA, t).fix.is_none());
    }

    #[test]
    fn rmc_only_receiver_emits_on_next_epoch() {
        let mut e = engine();
        let t = Instant::now();
        assert!(feed(&mut e, RMC, t).fix.is_none());
        let next = RMC.replace("120000.00", "120001.00");
        let fix = feed(&mut e, &next, t).fix.expect("previous epoch flushed");
        assert_eq!(fix.utc_time, NaiveTime::from_hms_opt(12, 0, 0));
        assert!(fix.valid);
        assert_eq!(fix.mode, FixMode::Fix2D);
    }

    #[test]
    fn idle_flush_emits_pending_epoch_once() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, GGA, t);
        assert!(e.flush_if_idle(t, Duration::from_secs(1)).is_none());
        let later = t + Duration::from_secs(2);
        assert!(e.flush_if_idle(later, Duration::from_secs(1)).is_some());
        assert!(e.flush_if_idle(later, Duration::from_secs(1)).is_none());
    }

    #[test]
    fn void_rmc_and_invalid_gga_are_not_valid() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, &RMC.replace(",A,5658", ",V,5658"), t);
        let fix = feed(&mut e, GGA, t).fix.unwrap();
        assert!(!fix.valid);
        assert_eq!(fix.mode, FixMode::NoFix);
        assert!(fix.position().is_none());

        let mut e = engine();
        feed(&mut e, RMC, t);
        let fix = feed(&mut e, &GGA.replace(",1,12,", ",0,12,"), t)
            .fix
            .unwrap();
        assert!(!fix.valid);
    }

    #[test]
    fn gst_accuracy_takes_precedence_and_expires() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, "GNGST,120000.00,1.0,2.0,1.0,0.0,3.0,4.0,5.0", t);
        feed(&mut e, RMC, t);
        let fix = feed(&mut e, GGA, t).fix.unwrap();
        assert_eq!(fix.accuracy_source, Some(AccuracySource::Gst));
        assert!((fix.accuracy_m.unwrap() - 5.0).abs() < 1e-9);

        let later = t + Duration::from_secs(10);
        feed(&mut e, &RMC.replace("120000.00", "120010.00"), later);
        let fix = feed(&mut e, &GGA.replace("120000.00", "120010.00"), later)
            .fix
            .unwrap();
        assert_eq!(fix.accuracy_source, Some(AccuracySource::HdopEstimate));
    }

    #[test]
    fn satellites_are_keyed_by_constellation_and_marked_used() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, "GPGSV,1,1,02,03,45,120,40,07,10,300,", t);
        feed(&mut e, "GAGSV,1,1,01,03,60,200,35,7", t);
        feed(&mut e, "GNGSA,A,3,03,,,,,,,,,,,,1.5,0.8,1.2,1", t);
        let sky = e.sky(t);
        assert_eq!(sky.in_view, 3);
        let gps3 = sky.satellites.iter().find(|s| s.id() == "G03").unwrap();
        let gal3 = sky.satellites.iter().find(|s| s.id() == "E03").unwrap();
        assert!(gps3.used);
        assert!(!gal3.used);
        assert_eq!(gal3.snr_dbhz, Some(35));
        assert_eq!(sky.hdop, Some(0.8));
        // Expiry
        assert_eq!(e.sky(t + Duration::from_secs(11)).in_view, 0);
    }

    #[test]
    fn vtg_both_formats_and_heading_rot() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, "GPVTG,054.7,T,034.4,M,005.5,N,010.8,K,A", t);
        feed(&mut e, "HEHDT,274.07,T", t);
        feed(&mut e, "HEROT,-30.0,A", t);
        feed(&mut e, &GGA.replace("120000.00", "120005.00"), t);
        let fix = e
            .flush_if_idle(t + Duration::from_secs(5), Duration::from_secs(1))
            .unwrap();
        assert!((fix.speed_mps.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(fix.course_deg, Some(54.7));
        assert_eq!(fix.heading_deg, Some(274.07));
        assert_eq!(fix.rate_of_turn_dps, Some(-0.5));

        let mut e = engine();
        feed(&mut e, "GPVTG,054.7,034.4,005.5,010.8", t);
        feed(&mut e, GGA, t);
        let fix = e
            .flush_if_idle(t + Duration::from_secs(5), Duration::from_secs(1))
            .unwrap();
        assert!((fix.speed_mps.unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn zda_supplies_date_for_gga_only_receivers() {
        let mut e = engine();
        let t = Instant::now();
        feed(&mut e, "GPZDA,120000.00,23,09,2026,00,00", t);
        feed(&mut e, GGA, t);
        let fix = e
            .flush_if_idle(t + Duration::from_secs(5), Duration::from_secs(1))
            .unwrap();
        assert_eq!(fix.time.unwrap().timestamp_millis(), 1_790_164_800_000);
    }

    #[test]
    fn txt_message_is_surfaced() {
        let mut e = engine();
        let out = feed(&mut e, "GPTXT,01,01,02,ANTSTATUS=OK", Instant::now());
        assert_eq!(out.text.as_deref(), Some("ANTSTATUS=OK"));
    }
}

/// End-to-end over real receiver captures (u-blox M8030, protocol 18).
#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::nmea::LineFramer;
    use crate::position::PositionEncoder;

    struct Run {
        fixes: Vec<Fix>,
        g86: usize,
        sky: Sky,
        texts: Vec<String>,
    }

    fn replay(name: &str) -> Run {
        replay_with(name, None)
    }

    fn replay_with(name: &str, hold: Option<crate::hold::HoldConfig>) -> Run {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(path).unwrap();
        let mut framer = LineFramer::new();
        let mut engine = Engine::new(Some(3), Duration::from_secs(10)).with_hold(hold);
        let mut g86 = PositionEncoder::new();
        let mut run = Run {
            fixes: vec![],
            g86: 0,
            sky: Sky::default(),
            texts: vec![],
        };
        let t = Instant::now();
        let mut lines = Vec::new();
        framer.push(&bytes, |l| lines.push(l.to_string()));
        for line in &lines {
            let s = Sentence::parse(line, true).expect("fixture lines are valid");
            run.g86 += usize::from(g86.sentence(line).is_some());
            let out = engine.ingest(&s, t);
            run.fixes.extend(out.fix);
            run.texts.extend(out.text);
        }
        run.fixes
            .extend(engine.flush_if_idle(t + Duration::from_secs(5), Duration::from_secs(1)));
        run.sky = engine.sky(t);
        run
    }

    #[test]
    fn cold_start_capture_has_no_valid_fix() {
        let run = replay("ublox-m8-cold-start.nmea");
        assert_eq!(run.fixes.len(), 4);
        assert!(
            run.fixes
                .iter()
                .all(|f| !f.valid && f.mode == FixMode::NoFix)
        );
        assert_eq!(run.g86, 0);
        assert!(run.texts.iter().any(|t| t.contains("UBX-M8030")));
        // GSV without elevation/azimuth during acquisition still lists the satellites.
        assert!(run.sky.in_view >= 6);
        assert_eq!(run.sky.used, 0);
    }

    #[test]
    fn fix_capture_produces_coherent_fixes() {
        let run = replay("ublox-m8-3d-fix-1hz.nmea");
        // The capture contains a real loss of fix right after a configuration
        // reload: 08:40:45 (RMC V, GGA quality 0) and 08:40:46 (GGA quality 0).
        assert_eq!(run.fixes.len(), 16);
        let invalid: Vec<_> = run.fixes.iter().filter(|f| !f.valid).collect();
        assert_eq!(invalid.len(), 2);
        assert!(
            invalid
                .iter()
                .all(|f| f.position().is_none() && f.mode == FixMode::NoFix && f.mgrs.is_none())
        );
        let valid: Vec<_> = run.fixes.iter().filter(|f| f.valid).collect();
        for f in &valid {
            let (lat, lon) = f.position().unwrap();
            assert!((lat - 56.9637).abs() < 0.001 && (lon - 24.2289).abs() < 0.001);
            assert!(f.time.is_some());
            assert!(f.hdop.is_some() && f.satellites_used.is_some());
            assert_eq!(f.accuracy_source, Some(AccuracySource::HdopEstimate));
        }
        let seqs: Vec<u64> = run.fixes.iter().map(|f| f.sequence).collect();
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
        // One G86 packet per valid epoch with matching RMC + GGA.
        assert_eq!(run.g86, valid.len());
        assert!(run.sky.in_view >= 10);
        assert!(run.sky.used >= 5, "GSA marks used satellites");
    }

    #[test]
    fn static_hold_steadies_the_stationary_capture() {
        fn spread(run: &Run) -> f64 {
            let held: Vec<(f64, f64)> =
                run.fixes.iter().skip(4).filter_map(Fix::position).collect();
            let (lat0, lon0) = held[0];
            held.iter()
                .map(|&(la, lo)| crate::telemetry::haversine(lat0, lon0, la, lo))
                .fold(0.0, f64::max)
        }
        let raw = replay("ublox-m8-3d-fix-1hz.nmea");
        let held = replay_with(
            "ublox-m8-3d-fix-1hz.nmea",
            Some(crate::hold::HoldConfig {
                speed_mps: 2.0 / 3.6,
                radius_m: 5.0,
            }),
        );
        assert!(
            held.fixes
                .iter()
                .skip(4)
                .filter(|f| f.valid)
                .all(|f| f.position_held)
        );
        assert!(
            spread(&held) < spread(&raw) / 2.0,
            "held {} m vs raw {} m",
            spread(&held),
            spread(&raw)
        );
        let distinct = |run: &Run| {
            let mut v: Vec<(u64, u64)> = run
                .fixes
                .iter()
                .skip(4)
                .filter_map(Fix::position)
                .map(|(a, b)| (a.to_bits(), b.to_bits()))
                .collect();
            v.dedup();
            v.len()
        };
        assert!(
            distinct(&held) <= 3,
            "held: {} distinct, raw: {}",
            distinct(&held),
            distinct(&raw)
        );
        assert!(distinct(&raw) >= 10);
        // The raw G86 packet path is unaffected by hold.
        assert_eq!(held.g86, raw.g86);
    }
}
