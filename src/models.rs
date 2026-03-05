use chrono::{NaiveDate, NaiveTime};
use std::collections::{BTreeMap, HashMap};

/// GNSS system type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GnssSystem {
    Gps,
    Glonass,
    Galileo,
    Beidou,
    Unknown,
}

/// GPS fix type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixType {
    NoFix,
    Fix2D,
    Fix3D,
    Unknown,
}

/// GPS fix quality indicator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixQuality {
    Invalid,
    GpsFix,
    DgpsFix,
    PpsFix,
    Rtk,
    FloatRtk,
    Estimated,
    Manual,
    Simulation,
}

/// Satellite information from GSV sentences
#[derive(Debug, Clone)]
pub struct SatelliteInfo {
    pub prn: u32,
    pub elevation: Option<i32>,
    pub azimuth: Option<i32>,
    pub snr: Option<i32>,
    pub system: GnssSystem,
    /// Wall-clock time the satellite was last reported by the receiver.
    /// Used to expire phantom entries without ever clearing mid-cycle.
    pub last_seen: std::time::Instant,
}

impl Default for SatelliteInfo {
    fn default() -> Self {
        Self {
            prn: 0,
            elevation: None,
            azimuth: None,
            snr: None,
            system: GnssSystem::Unknown,
            last_seen: std::time::Instant::now(),
        }
    }
}

/// Position and navigation data
#[derive(Debug, Clone, Default)]
pub struct NavigationData {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub speed_knots: Option<f64>,
    pub speed_kph: Option<f64>,
    pub course: Option<f64>,
    #[allow(dead_code)]
    pub magnetic_variation: Option<f64>,
    pub heading_rate: Option<f64>,      // degrees/second
    pub true_heading: Option<f64>,      // degrees (0-360)
    pub position_accuracy: Option<f64>, // meters (from GST)
}

/// GPS fix information
#[derive(Debug, Clone, Default)]
pub struct FixData {
    pub fix_type: Option<FixType>,
    pub fix_quality: Option<FixQuality>,
    pub satellites_used: Option<u32>,
    pub hdop: Option<f64>,
    pub vdop: Option<f64>,
    pub pdop: Option<f64>,
    pub time: Option<NaiveTime>,
    pub date: Option<NaiveDate>,
}

/// Complete GPS state
#[derive(Debug, Clone, Default)]
pub struct GpsData {
    pub navigation: NavigationData,
    pub fix: FixData,
    pub satellites: HashMap<u32, SatelliteInfo>,
    pub satellites_in_view: Option<u32>,
    pub messages: Vec<String>,
    pub raw_nmea_buffer: Vec<String>,
    pub last_update: Option<std::time::Instant>,
}

impl GpsData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a text message to the message log
    pub fn add_message(&mut self, msg: String) {
        self.messages.push(msg);
        // Keep last 100 messages
        if self.messages.len() > 100 {
            self.messages.remove(0);
        }
    }

    pub fn add_raw_nmea(&mut self, sentence: String) {
        self.raw_nmea_buffer.push(sentence);
        // Keep last 500 raw NMEA sentences
        if self.raw_nmea_buffer.len() > 500 {
            self.raw_nmea_buffer.remove(0);
        }
    }

    /// Upsert a satellite, always refreshing last_seen to now.
    pub fn update_satellite(&mut self, mut satellite: SatelliteInfo) {
        satellite.last_seen = std::time::Instant::now();
        self.satellites.insert(satellite.prn, satellite);
        self.last_update = Some(std::time::Instant::now());
    }

    /// Get satellites grouped by GNSS system (sorted by system)
    pub fn satellites_by_system(&self) -> BTreeMap<GnssSystem, Vec<&SatelliteInfo>> {
        let mut result: BTreeMap<GnssSystem, Vec<&SatelliteInfo>> = BTreeMap::new();
        for sat in self.satellites.values() {
            result.entry(sat.system).or_default().push(sat);
        }
        // Sort satellites within each system by PRN
        for sats in result.values_mut() {
            sats.sort_by_key(|s| s.prn);
        }
        result
    }

    /// Get satellites with valid signal strength
    #[allow(dead_code)]
    pub fn satellites_with_signal(&self) -> Vec<&SatelliteInfo> {
        self.satellites
            .values()
            .filter(|sat| sat.snr.is_some())
            .collect()
    }
}

/// MQTT connection status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Application state shared between tasks
#[derive(Debug, Clone)]
pub struct AppState {
    pub gps_data: GpsData,
    pub mqtt_status: MqttStatus,
    pub serial_connected: bool,
    /// Whether MQTT publishing is enabled
    pub mqtt_enabled: bool,
    /// Human-readable GPS connection string (e.g. "/dev/ttyUSB0 @ 9600 baud")
    pub connection_address: String,
    /// Human-readable MQTT broker address (e.g. "localhost:1883")
    pub mqtt_address: String,
    /// Shared atomic counter — incremented by the MQTT task on each publish.
    /// Read by the TUI without taking a lock.
    pub messages_published: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            gps_data: GpsData::new(),
            mqtt_status: MqttStatus::Disconnected,
            serial_connected: false,
            mqtt_enabled: true,
            connection_address: String::new(),
            mqtt_address: String::new(),
            messages_published: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

/// Telemetry metrics calculated from GPS data
#[derive(Debug, Clone, Default)]
pub struct TelemetryMetrics {
    pub longitudinal_accel: Option<f64>, // m/s² (+accel / -braking)
    pub longitudinal_g: Option<f64>,     // longitudinal_accel / 9.81
    pub lateral_accel: Option<f64>,      // m/s² (centripetal, via v×ω)
    pub lateral_g: Option<f64>,          // lateral_accel / 9.81
    pub combined_g: Option<f64>,         // √(long_g² + lat_g²)
    pub heading_rate: Option<f64>,       // degrees/second (yaw rate)
    pub distance_traveled: f64,          // odometer, metres
    pub max_speed_kph: Option<f64>,      // session maximum
    pub is_braking: bool,                // long_accel < -0.5 m/s²
}

/// Lap timing data
#[derive(Debug, Clone)]
pub struct LapData {
    pub lap_number: u32,
    pub lap_time_ms: Option<u64>, // Current/last lap time in milliseconds
    pub best_lap_ms: Option<u64>, // Best lap time in milliseconds
    pub current_lap_start_ms: Option<u64>, // Timestamp when current lap started
    pub sector_times_ms: Vec<Option<u64>>, // Sector times in milliseconds
}

impl Default for LapData {
    fn default() -> Self {
        Self {
            lap_number: 0,
            lap_time_ms: None,
            best_lap_ms: None,
            current_lap_start_ms: None,
            sector_times_ms: Vec::new(),
        }
    }
}

/// Track configuration for lap detection
#[derive(Debug, Clone)]
pub struct TrackConfig {
    pub mode: TrackConfigMode,
    pub start_finish: Option<TrackPoint>,
    pub sectors: Vec<TrackPoint>,
    pub learned_track: Vec<TrackPoint>, // Points recorded during learn mode
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackConfigMode {
    Manual,   // Manually configured start/finish
    Learn,    // Learning mode - recording track
    Gpx,      // Loaded from GPX file
    Disabled, // Lap timing disabled
}

/// A point on the track with geofence radius
#[derive(Debug, Clone)]
pub struct TrackPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: f64, // Geofence radius
    #[allow(dead_code)]
    pub name: String, // e.g., "Start/Finish", "Sector 1", etc.
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            mode: TrackConfigMode::Disabled,
            start_finish: None,
            sectors: Vec::new(),
            learned_track: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- GpsData ---

    #[test]
    fn test_gps_data_message_capped_at_100() {
        let mut gps = GpsData::new();
        for i in 0..110 {
            gps.add_message(format!("msg {}", i));
        }
        assert_eq!(gps.messages.len(), 100);
    }

    #[test]
    fn test_gps_data_raw_nmea_capped_at_500() {
        let mut gps = GpsData::new();
        for i in 0..510 {
            gps.add_raw_nmea(format!("$GNRMC,{}", i));
        }
        assert_eq!(gps.raw_nmea_buffer.len(), 500);
    }

    #[test]
    fn test_update_satellite_stores_and_sets_update_time() {
        let mut gps = GpsData::new();
        let sat = SatelliteInfo {
            prn: 7,
            elevation: Some(45),
            azimuth: Some(180),
            snr: Some(35),
            system: GnssSystem::Gps,
            ..Default::default()
        };
        gps.update_satellite(sat);
        assert_eq!(gps.satellites.len(), 1);
        assert_eq!(gps.satellites[&7].snr, Some(35));
        assert!(gps.last_update.is_some());
    }

    #[test]
    fn test_update_satellite_overwrites_existing_prn() {
        let mut gps = GpsData::new();
        gps.update_satellite(SatelliteInfo {
            prn: 1,
            elevation: Some(10),
            azimuth: None,
            snr: Some(20),
            system: GnssSystem::Gps,
            ..Default::default()
        });
        gps.update_satellite(SatelliteInfo {
            prn: 1,
            elevation: Some(15),
            azimuth: None,
            snr: Some(30),
            system: GnssSystem::Gps,
            ..Default::default()
        });
        assert_eq!(gps.satellites.len(), 1);
        assert_eq!(gps.satellites[&1].snr, Some(30));
    }

    #[test]
    fn test_satellites_by_system_groups_correctly() {
        let mut gps = GpsData::new();
        for prn in 1u32..=3 {
            gps.update_satellite(SatelliteInfo {
                prn,
                elevation: None,
                azimuth: None,
                snr: None,
                system: GnssSystem::Gps,
                ..Default::default()
            });
        }
        gps.update_satellite(SatelliteInfo {
            prn: 65,
            elevation: None,
            azimuth: None,
            snr: None,
            system: GnssSystem::Glonass,
            ..Default::default()
        });
        let by_system = gps.satellites_by_system();
        assert_eq!(by_system[&GnssSystem::Gps].len(), 3);
        assert_eq!(by_system[&GnssSystem::Glonass].len(), 1);
    }

    #[test]
    fn test_satellites_sorted_by_prn_within_system() {
        let mut gps = GpsData::new();
        for prn in &[5u32, 1, 3] {
            gps.update_satellite(SatelliteInfo {
                prn: *prn,
                elevation: None,
                azimuth: None,
                snr: None,
                system: GnssSystem::Gps,
                ..Default::default()
            });
        }
        let by_system = gps.satellites_by_system();
        let prns: Vec<u32> = by_system[&GnssSystem::Gps].iter().map(|s| s.prn).collect();
        assert_eq!(prns, vec![1, 3, 5]);
    }

    // --- AppState ---

    #[test]
    fn test_app_state_default_values() {
        let state = AppState::default();
        assert!(!state.serial_connected);
        assert!(state.mqtt_enabled);
        assert_eq!(
            state
                .messages_published
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(state.mqtt_status, MqttStatus::Disconnected);
        assert!(state.connection_address.is_empty());
        assert!(state.mqtt_address.is_empty());
    }

    // --- LapData ---

    #[test]
    fn test_lap_data_default() {
        let lap = LapData::default();
        assert_eq!(lap.lap_number, 0);
        assert!(lap.lap_time_ms.is_none());
        assert!(lap.best_lap_ms.is_none());
        assert!(lap.sector_times_ms.is_empty());
    }

    // --- TrackConfig ---

    #[test]
    fn test_track_config_default_mode_is_disabled() {
        let cfg = TrackConfig::default();
        assert_eq!(cfg.mode, TrackConfigMode::Disabled);
        assert!(cfg.start_finish.is_none());
        assert!(cfg.sectors.is_empty());
    }
}
