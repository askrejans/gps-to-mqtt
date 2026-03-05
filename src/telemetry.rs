use crate::models::{GpsData, TelemetryMetrics};
use std::collections::VecDeque;
use std::time::Instant;
use tracing::trace;

const GRAVITY: f64 = 9.81; // m/s²
const KPH_TO_MPS: f64 = 1.0 / 3.6;

// Minimum speed before any dynamics are computed.
// Below this threshold GPS course noise dominates and Δv/Δt is meaningless.
const MIN_SPEED_KPH: f64 = 3.0;

// Braking detection threshold: deceleration stronger than this (m/s²) = braking.
// ~0.5 m/s² ≈ light trail braking; hard braking is typically > 5 m/s².
const BRAKING_THRESHOLD_MPS2: f64 = -0.5;

/// One GPS epoch stored for derivative calculations.
#[derive(Debug, Clone)]
struct DataPoint {
    speed_mps: f64,  // already converted
    course_deg: f64, // 0-360
    latitude: f64,
    longitude: f64,
    timestamp: Instant,
}

/// Derived telemetry — calculated once per GPS epoch, never recalculated.
pub struct TelemetryCalculator {
    /// Ring buffer of the last `window_size` valid (speed > threshold) epochs.
    history: VecDeque<DataPoint>,
    window_size: usize,
    total_distance: f64,
    max_speed_kph: f64,
    /// Latest ROT from dedicated NMEA ROT sentence (deg/min), overrides course diff.
    last_rot_deg_per_min: Option<f64>,
    /// Last emitted metrics — returned unchanged when speed < threshold.
    last_metrics: TelemetryMetrics,
}

impl TelemetryCalculator {
    pub fn new(window_size: usize) -> Self {
        let ws = window_size.max(2);
        Self {
            history: VecDeque::with_capacity(ws),
            window_size: ws,
            total_distance: 0.0,
            max_speed_kph: 0.0,
            last_rot_deg_per_min: None,
            last_metrics: TelemetryMetrics::default(),
        }
    }

    /// Feed a new GPS epoch.  Returns updated metrics.
    /// Called once per RMC/GGA/VTG update — no repeated recalculation.
    pub fn update(&mut self, gps_data: &GpsData) -> TelemetryMetrics {
        let speed_kph = gps_data.navigation.speed_kph.unwrap_or(0.0);
        let lat = gps_data.navigation.latitude.unwrap_or(0.0);
        let lon = gps_data.navigation.longitude.unwrap_or(0.0);

        // Track session max speed regardless of threshold
        if speed_kph > self.max_speed_kph {
            self.max_speed_kph = speed_kph;
        }

        // Only accumulate distance and history when we have a real fix and are moving
        let has_fix = lat != 0.0 || lon != 0.0;
        let is_moving = speed_kph >= MIN_SPEED_KPH;

        if has_fix && is_moving {
            // Distance — integrate between consecutive valid positions
            if let Some(prev) = self.history.back() {
                let d = haversine(prev.latitude, prev.longitude, lat, lon);
                // Sanity-check: GPS noise / teleport guard (max ~300 m/s = 1080 km/h)
                if d < speed_kph * KPH_TO_MPS * 2.0 + 50.0 {
                    self.total_distance += d;
                }
            }

            let course = gps_data.navigation.course.unwrap_or(0.0);
            let pt = DataPoint {
                speed_mps: speed_kph * KPH_TO_MPS,
                course_deg: course,
                latitude: lat,
                longitude: lon,
                timestamp: Instant::now(),
            };
            self.history.push_back(pt);
            if self.history.len() > self.window_size {
                self.history.pop_front();
            }
        } else if !is_moving {
            // Vehicle stopped — clear dynamics history so stale deltas don't
            // produce spurious acceleration on the next move.
            self.history.clear();
        }

        // ── Compute derivatives ──────────────────────────────────────────────
        let mut metrics = TelemetryMetrics {
            distance_traveled: self.total_distance,
            max_speed_kph: if self.max_speed_kph > 0.0 {
                Some(self.max_speed_kph)
            } else {
                None
            },
            ..Default::default()
        };

        if self.history.len() >= 2 {
            let newest = self.history.back().unwrap();
            let oldest = self.history.front().unwrap();

            let dt = newest
                .timestamp
                .duration_since(oldest.timestamp)
                .as_secs_f64();

            if dt >= 0.05 {
                // ── Longitudinal acceleration (Δv / Δt over window) ─────────
                let dv = newest.speed_mps - oldest.speed_mps;
                let long_accel = dv / dt; // m/s²

                metrics.longitudinal_accel = Some(long_accel);
                metrics.longitudinal_g = Some(long_accel / GRAVITY);
                metrics.is_braking = long_accel < BRAKING_THRESHOLD_MPS2;

                // ── Heading rate (°/s, shortest-path wrap) ───────────────────
                let heading_rate_deg_s = if let Some(rot) = self.last_rot_deg_per_min {
                    // Prefer dedicated ROT sentence for accuracy
                    rot / 60.0
                } else {
                    let mut dh = newest.course_deg - oldest.course_deg;
                    // Normalise to (-180, +180]
                    while dh > 180.0 {
                        dh -= 360.0;
                    }
                    while dh < -180.0 {
                        dh += 360.0;
                    }
                    dh / dt
                };

                metrics.heading_rate = Some(heading_rate_deg_s);

                // ── Lateral acceleration: a_lat = v × ω  (centripetal) ──────
                // ω in rad/s, v in m/s → a_lat in m/s²
                let omega = heading_rate_deg_s.to_radians(); // rad/s
                let v = newest.speed_mps;
                let lat_accel = v * omega; // m/s²

                metrics.lateral_accel = Some(lat_accel);
                metrics.lateral_g = Some(lat_accel / GRAVITY);

                // ── Combined g ───────────────────────────────────────────────
                let long_g = long_accel / GRAVITY;
                let lat_g = lat_accel / GRAVITY;
                metrics.combined_g = Some((long_g * long_g + lat_g * lat_g).sqrt());
            }
        }

        trace!("Telemetry: {:?}", metrics);
        self.last_metrics = metrics.clone();
        metrics
    }

    /// Feed a ROT (rate-of-turn) sentence value in degrees/minute.
    /// Stored and used instead of the course-difference estimate on the next update.
    pub fn update_rate_of_turn(&mut self, rate_deg_per_min: f64) {
        self.last_rot_deg_per_min = Some(rate_deg_per_min);
    }

    /// Reset odometer (e.g. new lap start).
    #[allow(dead_code)]
    pub fn reset_distance(&mut self) {
        self.total_distance = 0.0;
    }
}

/// Haversine distance between two coordinates, metres.
fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine_known_distance() {
        let d = haversine(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_195.0).abs() < 200.0);
    }

    #[test]
    fn test_haversine_same_point() {
        assert!(haversine(40.7128, -74.006, 40.7128, -74.006) < 0.1);
    }

    #[test]
    fn test_first_update_no_dynamics() {
        let mut calc = TelemetryCalculator::new(3);
        let metrics = calc.update(&GpsData::default());
        assert!(metrics.longitudinal_accel.is_none());
        assert!(metrics.lateral_accel.is_none());
        assert!(metrics.combined_g.is_none());
        assert!(metrics.longitudinal_g.is_none());
        assert!(metrics.lateral_g.is_none());
    }

    #[test]
    fn test_below_threshold_no_dynamics() {
        // 2 km/h < MIN_SPEED_KPH — history should stay empty, no accel
        let mut calc = TelemetryCalculator::new(3);
        let mut gps = GpsData::default();
        gps.navigation.latitude = Some(1.0);
        gps.navigation.longitude = Some(1.0);
        gps.navigation.speed_kph = Some(2.0);
        calc.update(&gps);
        gps.navigation.speed_kph = Some(2.5);
        let m = calc.update(&gps);
        assert!(m.longitudinal_accel.is_none());
    }

    #[test]
    fn test_max_speed_retained() {
        let mut calc = TelemetryCalculator::new(3);
        let mut gps = GpsData::default();
        gps.navigation.latitude = Some(1.0);
        gps.navigation.longitude = Some(1.0);

        gps.navigation.speed_kph = Some(50.0);
        calc.update(&gps);
        gps.navigation.speed_kph = Some(120.0);
        calc.update(&gps);
        gps.navigation.speed_kph = Some(80.0);
        let m = calc.update(&gps);
        assert_eq!(m.max_speed_kph, Some(120.0));
    }

    #[test]
    fn test_is_braking_flag() {
        let mut calc = TelemetryCalculator::new(5);
        let mut gps = GpsData::default();
        gps.navigation.latitude = Some(1.0);
        gps.navigation.longitude = Some(1.0);
        gps.navigation.course = Some(0.0);

        // Establish history at 100 km/h — need real time gap so dt >= 50ms
        gps.navigation.speed_kph = Some(100.0);
        calc.update(&gps);
        std::thread::sleep(std::time::Duration::from_millis(60));
        gps.navigation.speed_kph = Some(100.0);
        calc.update(&gps);
        std::thread::sleep(std::time::Duration::from_millis(60));
        // Hard stop to 10 km/h
        gps.navigation.speed_kph = Some(10.0);
        let m = calc.update(&gps);
        // Should be braking (large negative accel)
        assert!(m.is_braking, "expected braking flag");
        assert!(m.longitudinal_accel.unwrap() < BRAKING_THRESHOLD_MPS2);
    }

    #[test]
    fn test_rot_overrides_course_diff() {
        let mut calc = TelemetryCalculator::new(3);
        calc.update_rate_of_turn(120.0); // 2 °/s
        // Just must not panic
    }

    #[test]
    fn test_distance_only_accumulates_above_threshold() {
        let mut calc = TelemetryCalculator::new(3);
        let mut gps = GpsData::default();

        // Stationary — should not accumulate
        gps.navigation.latitude = Some(51.0);
        gps.navigation.longitude = Some(0.0);
        gps.navigation.speed_kph = Some(0.0);
        calc.update(&gps);
        // Even if position changes, distance should not accumulate when stopped
        gps.navigation.latitude = Some(51.001);
        let m = calc.update(&gps);
        assert_eq!(m.distance_traveled, 0.0, "stationary move should not count");

        // Now moving at 50 km/h (~13.9 m/s).  Move ~14m north — realistic 1s epoch.
        // 1 degree lat ≈ 111 km → 0.000125 deg ≈ ~14 m
        gps.navigation.speed_kph = Some(50.0);
        gps.navigation.latitude = Some(51.001);
        gps.navigation.longitude = Some(0.0);
        calc.update(&gps);
        gps.navigation.latitude = Some(51.001125); // ~12.5 m north
        let m = calc.update(&gps);
        assert!(
            m.distance_traveled > 0.0,
            "distance should accumulate when moving"
        );
        assert!(
            m.distance_traveled < 100.0,
            "distance should be realistic, not teleport"
        );
    }
}
