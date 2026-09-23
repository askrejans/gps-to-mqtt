//! Coherent live position packets for G86. RMC and GGA must describe the same fix.
use crate::parser::{parse_coordinate, parse_date, parse_time};
use chrono::{NaiveTime, Utc};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Position {
    schema: u8,
    source: &'static str,
    boot_id: String,
    sequence: u64,
    timestamp_ms: i64,
    latitude: f64,
    longitude: f64,
    speed_mps: f64,
    heading_deg: f64,
    accuracy_m: f64,
    altitude_m: f64,
    accuracy_source: &'static str,
}

pub struct PositionEncoder {
    boot: String,
    sequence: u64,
    rmc: Option<(NaiveTime, Position)>,
    gga: Option<(NaiveTime, f64, f64, f64, f64)>,
    last_timestamp: i64,
}

impl PositionEncoder {
    pub fn new() -> Self {
        Self {
            boot: format!("{}-{}", std::process::id(), Utc::now().timestamp_micros()),
            sequence: 0,
            rmc: None,
            gga: None,
            last_timestamp: 0,
        }
    }

    /// Reader already checks checksums. Recheck here so callers cannot bypass it.
    pub fn sentence(&mut self, sentence: &str) -> Option<String> {
        if !sentence.is_ascii() || sentence.len() > 1024 {
            return None;
        }
        let (body, checksum) = sentence.strip_prefix('$')?.split_once('*')?;
        if checksum.len() != 2
            || u8::from_str_radix(checksum, 16).ok()? != body.bytes().fold(0, |a, b| a ^ b)
        {
            return None;
        }
        let p: Vec<_> = body.split(',').collect();
        let kind = p.first()?.get(2..)?;
        match kind {
            "RMC" => {
                self.rmc = None;
                if p.len() < 12
                    || p[2] != "A"
                    || !["N", "S"].contains(&p[4])
                    || !["E", "W"].contains(&p[6])
                {
                    return None;
                }
                // Reject dead reckoning, manual/simulation and invalid modes.
                if let Some(mode) = p.get(12) {
                    if !["", "A", "D", "F", "R", "P"].contains(mode) {
                        return None;
                    }
                }
                let time = parse_time(p[1]).ok()?;
                let timestamp = parse_date(p[9])
                    .ok()?
                    .and_time(time)
                    .and_utc()
                    .timestamp_millis();
                let latitude = parse_coordinate(p[3], p[4]).ok()?;
                let longitude = parse_coordinate(p[5], p[6]).ok()?;
                let speed = p[7].parse::<f64>().ok()? * 1.852 / 3.6;
                let heading = if p[8].is_empty() && speed < 0.5 {
                    0.0
                } else {
                    p[8].parse::<f64>().ok()?
                };
                if !latitude.is_finite()
                    || latitude.abs() > 90.0
                    || !longitude.is_finite()
                    || longitude.abs() > 180.0
                    || !speed.is_finite()
                    || !(0.0..=120.0).contains(&speed)
                    || !heading.is_finite()
                    || !(0.0..=360.0).contains(&heading)
                {
                    return None;
                }
                self.rmc = Some((
                    time,
                    Position {
                        schema: 1,
                        source: "nmea",
                        boot_id: self.boot.clone(),
                        sequence: 0,
                        timestamp_ms: timestamp,
                        latitude,
                        longitude,
                        speed_mps: speed,
                        heading_deg: heading % 360.0,
                        accuracy_m: 0.0,
                        altitude_m: 0.0,
                        accuracy_source: "hdop-estimate",
                    },
                ));
            }
            "GGA" => {
                self.gga = None;
                if p.len() < 11 || !["1", "2", "3", "4", "5"].contains(&p[6]) || p[10] != "M" {
                    return None;
                }
                let time = parse_time(p[1]).ok()?;
                let latitude = parse_coordinate(p[2], p[3]).ok()?;
                let longitude = parse_coordinate(p[4], p[5]).ok()?;
                let hdop = p[8].parse::<f64>().ok()?;
                let altitude = p[9].parse::<f64>().ok()?;
                if !hdop.is_finite()
                    || hdop <= 0.0
                    || hdop > 20.0
                    || !altitude.is_finite()
                    || !(-1000.0..=20000.0).contains(&altitude)
                {
                    return None;
                }
                // HDOP is dimensionless. A conservative 5 m UERE is an estimate,
                // not a claim of receiver-reported horizontal accuracy.
                self.gga = Some((time, (hdop * 5.0).max(3.0), altitude, latitude, longitude));
            }
            _ => return None,
        }
        let (time, position) = self.rmc.as_ref()?;
        let (quality_time, accuracy, altitude, latitude, longitude) = self.gga?;
        if *time != quality_time
            || position.timestamp_ms <= self.last_timestamp
            || (position.latitude - latitude).abs() > 0.00002
            || (position.longitude - longitude).abs() > 0.00002
        {
            return None;
        }
        let mut position = position.clone();
        position.accuracy_m = accuracy;
        position.altitude_m = altitude;
        position.sequence = self.sequence;
        self.sequence += 1;
        self.last_timestamp = position.timestamp_ms;
        serde_json::to_string(&position).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nmea(body: &str) -> String {
        format!("${body}*{:02X}", body.bytes().fold(0, |a, b| a ^ b))
    }
    const RMC: &str = "GNRMC,120000.100,A,5658.0000,N,02414.0000,E,50.0,85.0,230926,,,A";
    const GGA: &str = "GNGGA,120000.100,5658.0000,N,02414.0000,E,1,12,0.8,12.0,M,0.0,M,,";
    #[test]
    fn coherent_fix_in_either_order_and_duplicate_suppression() {
        for reverse in [false, true] {
            let mut encoder = PositionEncoder::new();
            let (a, b) = if reverse { (GGA, RMC) } else { (RMC, GGA) };
            assert!(encoder.sentence(&nmea(a)).is_none());
            let packet: serde_json::Value =
                serde_json::from_str(&encoder.sentence(&nmea(b)).unwrap()).unwrap();
            assert_eq!(packet["accuracyM"], 4.0);
            assert_eq!(packet["timestampMs"], 1790164800100_i64);
            assert!((packet["speedMps"].as_f64().unwrap() - 25.722222).abs() < 0.0001);
            assert!(encoder.sentence(&nmea(RMC)).is_none());
            assert!(encoder.sentence(&nmea(GGA)).is_none());
        }
    }
    #[test]
    fn rejects_invalid_stale_quality_simulation_and_bad_checksum() {
        for bad in [
            RMC.replace(",A,", ",V,"),
            RMC.replace(",,,A", ",,,S"),
            RMC.replace(",N,", ",X,"),
            RMC.replace("5658.0000", "5668.0000"),
            RMC.replace("5658.0000", "é658.0000"),
            RMC.replace("120000.100", "é20000.100"),
        ] {
            let mut e = PositionEncoder::new();
            e.sentence(&nmea(GGA));
            assert!(e.sentence(&nmea(&bad)).is_none());
        }
        let mut e = PositionEncoder::new();
        e.sentence(&nmea(RMC));
        assert!(
            e.sentence(&nmea(&GGA.replace("120000.100", "115959.100")))
                .is_none()
        );
        assert!(
            e.sentence(&nmea(&GGA.replace(",1,12,", ",0,12,")))
                .is_none()
        );
        assert!(e.sentence(&format!("${GGA}*00")).is_none());
        assert!(
            e.sentence(&nmea(&GGA.replace("5658.0000", "5659.0000")))
                .is_none()
        );
    }
}
