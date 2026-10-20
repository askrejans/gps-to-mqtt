//! Vehicle dynamics derived from consecutive fixes.
//!
//! Computed exactly once per emitted [`Fix`], using receiver UTC time for Δt
//! (host arrival time only when the receiver gives no date/time), so serial
//! buffering or network jitter does not distort derivatives.

use crate::models::{Fix, Telemetry};
use std::collections::VecDeque;

const GRAVITY: f64 = 9.806_65;
/// Below this speed course noise dominates and Δv/Δt is meaningless.
const MIN_SPEED_MPS: f64 = 3.0 / 3.6;
/// Deceleration stronger than this (m/s²) is reported as braking.
const BRAKING_THRESHOLD_MPS2: f64 = -0.5;
/// Gaps longer than this restart the derivative window.
const MAX_GAP_S: f64 = 2.5;

#[derive(Debug, Clone)]
struct Sample {
    /// Milliseconds (integer: f64 seconds since 1970 lose sub-µs precision).
    t_ms: i64,
    speed: f64,
    course: f64,
    lat: f64,
    lon: f64,
}

pub struct TelemetryCalculator {
    history: VecDeque<Sample>,
    window: usize,
    distance_m: f64,
    max_speed_mps: f64,
}

impl TelemetryCalculator {
    pub fn new(window: usize) -> Self {
        let window = window.max(2);
        Self {
            history: VecDeque::with_capacity(window),
            window,
            distance_m: 0.0,
            max_speed_mps: 0.0,
        }
    }

    pub fn update(&mut self, fix: &Fix) -> Telemetry {
        let mut out = Telemetry {
            distance_m: self.distance_m,
            max_speed_mps: self.max_speed_mps,
            ..Telemetry::default()
        };
        let (Some((lat, lon)), Some(speed)) = (fix.position(), fix.speed_mps) else {
            self.history.clear();
            return out;
        };
        if !fix.quality.is_measured() {
            self.history.clear();
            return out;
        }

        self.max_speed_mps = self.max_speed_mps.max(speed);
        out.max_speed_mps = self.max_speed_mps;

        let t_ms = fix
            .time
            .map(|t| t.timestamp_millis())
            .unwrap_or(fix.received_ms);

        if speed < MIN_SPEED_MPS {
            // Stationary: clear so stale deltas don't produce a spike on pull-away.
            self.history.clear();
            return out;
        }

        if let Some(prev) = self.history.back() {
            let dt = (t_ms - prev.t_ms) as f64 / 1000.0;
            if dt <= 0.0 || dt > MAX_GAP_S {
                self.history.clear();
            } else {
                let d = haversine(prev.lat, prev.lon, lat, lon);
                // Teleport guard: reject jumps beyond 2× the plausible travel.
                if d <= speed.max(prev.speed) * dt * 2.0 + 5.0 {
                    self.distance_m += d;
                }
            }
        }
        out.distance_m = self.distance_m;

        self.history.push_back(Sample {
            t_ms,
            speed,
            course: fix.course_deg.unwrap_or(0.0),
            lat,
            lon,
        });
        while self.history.len() > self.window {
            self.history.pop_front();
        }

        let (Some(oldest), Some(newest)) = (self.history.front(), self.history.back()) else {
            return out;
        };
        let dt = (newest.t_ms - oldest.t_ms) as f64 / 1000.0;
        if self.history.len() < 2 || dt < 0.05 {
            return out;
        }

        let long = (newest.speed - oldest.speed) / dt;
        let yaw = fix.rate_of_turn_dps.unwrap_or_else(|| {
            let mut dh = newest.course - oldest.course;
            dh = (dh + 180.0).rem_euclid(360.0) - 180.0;
            dh / dt
        });
        let lat_accel = newest.speed * yaw.to_radians();

        out.longitudinal_accel_mps2 = Some(long);
        out.longitudinal_g = Some(long / GRAVITY);
        out.lateral_accel_mps2 = Some(lat_accel);
        out.lateral_g = Some(lat_accel / GRAVITY);
        out.combined_g = Some(long.hypot(lat_accel) / GRAVITY);
        out.yaw_rate_dps = Some(yaw);
        out.braking = long < BRAKING_THRESHOLD_MPS2;
        out
    }
}

/// Great-circle distance in metres.
pub fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_008.8;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FixQuality;
    use chrono::{TimeZone, Utc};

    fn fix(ms: i64, lat: f64, speed_mps: f64, course: f64) -> Fix {
        Fix {
            time: Some(Utc.timestamp_millis_opt(1_790_164_800_000 + ms).unwrap()),
            valid: true,
            quality: FixQuality::Gps,
            latitude: Some(lat),
            longitude: Some(24.0),
            speed_mps: Some(speed_mps),
            course_deg: Some(course),
            ..Fix::default()
        }
    }

    #[test]
    fn haversine_known_distance() {
        assert!((haversine(0.0, 0.0, 1.0, 0.0) - 111_195.0).abs() < 200.0);
        assert!(haversine(40.0, -74.0, 40.0, -74.0) < 0.01);
    }

    #[test]
    fn longitudinal_accel_uses_receiver_time() {
        let mut c = TelemetryCalculator::new(2);
        assert!(
            c.update(&fix(0, 56.0, 10.0, 90.0))
                .longitudinal_accel_mps2
                .is_none()
        );
        let t = c.update(&fix(100, 56.0000090, 11.0, 90.0));
        assert!((t.longitudinal_accel_mps2.unwrap() - 10.0).abs() < 1e-6);
        assert!(!t.braking);
        let t = c.update(&fix(200, 56.0000180, 10.0, 90.0));
        assert!(t.braking);
        assert!(t.distance_m > 1.5 && t.distance_m < 2.5);
    }

    #[test]
    fn lateral_from_course_change_with_wrap() {
        let mut c = TelemetryCalculator::new(2);
        c.update(&fix(0, 56.0, 20.0, 359.0));
        let t = c.update(&fix(1000, 56.00018, 20.0, 9.0));
        assert!((t.yaw_rate_dps.unwrap() - 10.0).abs() < 1e-9);
        let expected = 20.0 * 10f64.to_radians();
        assert!((t.lateral_accel_mps2.unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn rot_overrides_course_difference() {
        let mut c = TelemetryCalculator::new(2);
        c.update(&fix(0, 56.0, 20.0, 0.0));
        let mut f = fix(1000, 56.00018, 20.0, 50.0);
        f.rate_of_turn_dps = Some(-2.0);
        assert_eq!(c.update(&f).yaw_rate_dps, Some(-2.0));
    }

    #[test]
    fn stationary_invalid_and_gaps_reset_dynamics() {
        let mut c = TelemetryCalculator::new(3);
        c.update(&fix(0, 56.0, 20.0, 0.0));
        assert!(
            c.update(&fix(100, 56.0, 0.1, 0.0))
                .longitudinal_accel_mps2
                .is_none()
        );
        c.update(&fix(200, 56.0, 20.0, 0.0));
        assert!(
            c.update(&fix(5000, 56.0, 25.0, 0.0))
                .longitudinal_accel_mps2
                .is_none()
        );
        let mut invalid = fix(5100, 56.0, 30.0, 0.0);
        invalid.valid = false;
        assert!(c.update(&invalid).longitudinal_accel_mps2.is_none());
        let mut dr = fix(5200, 56.0, 30.0, 0.0);
        dr.quality = FixQuality::Estimated;
        assert!(c.update(&dr).longitudinal_accel_mps2.is_none());
    }

    #[test]
    fn teleport_is_not_counted_and_max_speed_kept() {
        let mut c = TelemetryCalculator::new(3);
        c.update(&fix(0, 56.0, 30.0, 0.0));
        let t = c.update(&fix(100, 57.0, 10.0, 0.0));
        assert_eq!(t.distance_m, 0.0);
        assert_eq!(t.max_speed_mps, 30.0);
    }
}
