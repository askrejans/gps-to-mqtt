use crate::models::{GpsData, TelemetryMetrics};
use std::collections::VecDeque;
use std::time::Instant;
use tracing::trace;

const GRAVITY: f64 = 9.81; // m/s²
const BRAKING_THRESHOLD: f64 = -2.0; // m/s² - threshold for braking detection
const KPH_TO_MPS: f64 = 1.0 / 3.6; // Conversion factor from km/h to m/s

/// Historical data point for smoothing calculations
#[derive(Debug, Clone)]
struct DataPoint {
    speed_kph: f64,
    course: f64,
    latitude: f64,
    longitude: f64,
    timestamp: Instant,
}

/// Calculator for derived telemetry metrics
pub struct TelemetryCalculator {
    history: VecDeque<DataPoint>,
    window_size: usize,
    total_distance: f64,
    max_speed_kph: Option<f64>,
    last_rate_of_turn: Option<f64>, // From ROT sentence (degrees/minute)
}

impl TelemetryCalculator {
    /// Create a new telemetry calculator with specified smoothing window size
    pub fn new(window_size: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(window_size),
            window_size,
            total_distance: 0.0,
            max_speed_kph: None,
            last_rate_of_turn: None,
        }
    }

    /// Update with new GPS data and return calculated telemetry metrics
    pub fn update(&mut self, gps_data: &GpsData) -> TelemetryMetrics {
        let now = Instant::now();

        // Extract current values
        let speed_kph = gps_data.navigation.speed_kph.unwrap_or(0.0);
        let course = gps_data.navigation.course.unwrap_or(0.0);
        let latitude = gps_data.navigation.latitude.unwrap_or(0.0);
        let longitude = gps_data.navigation.longitude.unwrap_or(0.0);

        // Track maximum speed
        if speed_kph > self.max_speed_kph.unwrap_or(0.0) {
            self.max_speed_kph = Some(speed_kph);
        }

        // Create current data point
        let current = DataPoint {
            speed_kph,
            course,
            latitude,
            longitude,
            timestamp: now,
        };

        // Calculate metrics if we have historical data
        let metrics = if !self.history.is_empty() {
            self.calculate_metrics(&current)
        } else {
            TelemetryMetrics {
                distance_traveled: self.total_distance,
                max_speed_kph: self.max_speed_kph,
                ..Default::default()
            }
        };

        // Update distance traveled
        if let Some(last) = self.history.back() {
            let distance = calculate_distance(
                last.latitude,
                last.longitude,
                current.latitude,
                current.longitude,
            );
            self.total_distance += distance;
        }

        // Add current point to history
        self.history.push_back(current);

        // Maintain window size
        if self.history.len() > self.window_size {
            self.history.pop_front();
        }

        metrics
    }

    /// Update rate of turn from ROT sentence
    #[allow(dead_code)]
    pub fn update_rate_of_turn(&mut self, rate_deg_per_min: f64) {
        self.last_rate_of_turn = Some(rate_deg_per_min);
    }

    /// Reset distance counter (e.g., at start of new lap)
    #[allow(dead_code)]
    pub fn reset_distance(&mut self) {
        self.total_distance = 0.0;
    }

    /// Calculate telemetry metrics from historical data
    fn calculate_metrics(&self, current: &DataPoint) -> TelemetryMetrics {
        let mut metrics = TelemetryMetrics {
            distance_traveled: self.total_distance,
            max_speed_kph: self.max_speed_kph,
            ..Default::default()
        };

        // Need at least 2 points for calculations
        if self.history.len() < 2 {
            return metrics;
        }

        // Calculate longitudinal acceleration (from speed changes)
        if let Some(accel) = self.calculate_longitudinal_acceleration(current) {
            metrics.longitudinal_accel = Some(accel);
            metrics.is_braking = accel < BRAKING_THRESHOLD;
        }

        // Calculate heading rate (from course changes)
        if let Some(heading_rate) = self.calculate_heading_rate(current) {
            metrics.heading_rate = Some(heading_rate);

            // Calculate lateral acceleration if we have speed and heading rate
            let speed_mps = current.speed_kph * KPH_TO_MPS;
            if speed_mps > 0.5 {
                // Only calculate if moving
                let heading_rate_rad_per_sec = heading_rate.to_radians() / 1.0;
                let lateral_accel = speed_mps * heading_rate_rad_per_sec;
                metrics.lateral_accel = Some(lateral_accel);
            }
        }

        // If we have ROT data, use it for more accurate lateral acceleration
        if let Some(rot_deg_per_min) = self.last_rate_of_turn {
            let speed_mps = current.speed_kph * KPH_TO_MPS;
            if speed_mps > 0.5 {
                let rot_deg_per_sec = rot_deg_per_min / 60.0;
                let rot_rad_per_sec = rot_deg_per_sec.to_radians();
                let lateral_accel = speed_mps * rot_rad_per_sec;
                metrics.lateral_accel = Some(lateral_accel);
            }
        }

        // Calculate combined g-force
        if let (Some(long), Some(lat)) = (metrics.longitudinal_accel, metrics.lateral_accel) {
            let combined_accel = (long * long + lat * lat).sqrt();
            metrics.combined_g = Some(combined_accel / GRAVITY);
        }

        trace!("Telemetry metrics: {:?}", metrics);
        metrics
    }

    /// Calculate longitudinal acceleration using 3-point smoothing
    fn calculate_longitudinal_acceleration(&self, current: &DataPoint) -> Option<f64> {
        if self.history.is_empty() {
            return None;
        }

        let last = self.history.back()?;
        let time_delta = current
            .timestamp
            .duration_since(last.timestamp)
            .as_secs_f64();

        if time_delta < 0.001 {
            return None; // Too small time delta
        }

        // Convert speeds to m/s
        let speed_current = current.speed_kph * KPH_TO_MPS;
        let speed_last = last.speed_kph * KPH_TO_MPS;

        // Calculate acceleration
        let accel = (speed_current - speed_last) / time_delta;

        // Apply smoothing if we have enough history
        if self.history.len() >= 2 {
            let prev = &self.history[self.history.len() - 2];
            let time_delta_prev = last.timestamp.duration_since(prev.timestamp).as_secs_f64();

            if time_delta_prev > 0.001 {
                let speed_prev = prev.speed_kph * KPH_TO_MPS;
                let accel_prev = (speed_last - speed_prev) / time_delta_prev;

                // 3-point moving average
                return Some((accel + accel_prev) / 2.0);
            }
        }

        Some(accel)
    }

    /// Calculate heading change rate in degrees per second
    fn calculate_heading_rate(&self, current: &DataPoint) -> Option<f64> {
        if self.history.is_empty() {
            return None;
        }

        let last = self.history.back()?;
        let time_delta = current
            .timestamp
            .duration_since(last.timestamp)
            .as_secs_f64();

        if time_delta < 0.001 {
            return None;
        }

        // Calculate heading change, accounting for 360-degree wrap
        let mut heading_change = current.course - last.course;

        // Normalize to -180 to +180
        if heading_change > 180.0 {
            heading_change -= 360.0;
        } else if heading_change < -180.0 {
            heading_change += 360.0;
        }

        let heading_rate = heading_change / time_delta;

        // Apply smoothing if we have enough history
        if self.history.len() >= 2 {
            let prev = &self.history[self.history.len() - 2];
            let time_delta_prev = last.timestamp.duration_since(prev.timestamp).as_secs_f64();

            if time_delta_prev > 0.001 {
                let mut heading_change_prev = last.course - prev.course;

                // Normalize
                if heading_change_prev > 180.0 {
                    heading_change_prev -= 360.0;
                } else if heading_change_prev < -180.0 {
                    heading_change_prev += 360.0;
                }

                let heading_rate_prev = heading_change_prev / time_delta_prev;

                // 3-point moving average
                return Some((heading_rate + heading_rate_prev) / 2.0);
            }
        }

        Some(heading_rate)
    }
}

/// Calculate distance between two GPS coordinates using Haversine formula
/// Returns distance in meters
fn calculate_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS: f64 = 6371000.0; // Earth's radius in meters

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin() * (delta_lat / 2.0).sin()
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin() * (delta_lon / 2.0).sin();

    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS * c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_distance() {
        // Test distance between two known points (approximately 111 km apart)
        let distance = calculate_distance(0.0, 0.0, 1.0, 0.0);
        assert!((distance - 111195.0).abs() < 100.0); // Should be ~111 km
    }

    #[test]
    fn test_haversine_same_point() {
        let distance = calculate_distance(40.7128, -74.0060, 40.7128, -74.0060);
        assert!(distance < 0.1); // Should be essentially zero
    }

    #[test]
    fn test_first_update_no_accel_or_lateral() {
        let mut calc = TelemetryCalculator::new(3);
        let gps = GpsData::default();
        let metrics = calc.update(&gps);
        assert!(metrics.longitudinal_accel.is_none());
        assert!(metrics.lateral_accel.is_none());
        assert!(metrics.combined_g.is_none());
    }

    #[test]
    fn test_max_speed_increases_and_never_decreases() {
        let mut calc = TelemetryCalculator::new(3);
        let mut gps = GpsData::default();

        gps.navigation.speed_kph = Some(50.0);
        calc.update(&gps);

        gps.navigation.speed_kph = Some(120.0);
        let metrics = calc.update(&gps);
        assert_eq!(metrics.max_speed_kph, Some(120.0));

        // Speed drops — max must be retained
        gps.navigation.speed_kph = Some(80.0);
        let metrics = calc.update(&gps);
        assert_eq!(metrics.max_speed_kph, Some(120.0));
    }

    #[test]
    fn test_update_rate_of_turn_does_not_panic() {
        let mut calc = TelemetryCalculator::new(3);
        calc.update_rate_of_turn(120.0); // 120 deg/min — just must not panic
    }

    #[test]
    fn test_distance_accumulates_across_updates() {
        let mut calc = TelemetryCalculator::new(3);
        let mut gps = GpsData::default();

        gps.navigation.latitude = Some(0.0);
        gps.navigation.longitude = Some(0.0);
        calc.update(&gps);

        // Move ~111 km north
        gps.navigation.latitude = Some(1.0);
        calc.update(&gps);

        // Third update — now total_distance was committed after the 2nd call
        gps.navigation.latitude = Some(1.0); // stay still
        let metrics = calc.update(&gps);
        assert!(
            metrics.distance_traveled > 100_000.0,
            "expected > 100 km, got {}",
            metrics.distance_traveled
        );
    }
}
