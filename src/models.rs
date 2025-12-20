use chrono::{NaiveDate, NaiveTime};
use std::collections::{BTreeMap, HashMap};

/// Represents the operational mode of the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Interactive TUI mode with dashboard
    Tui,
    /// CLI mode with minimal output
    Cli,
    /// Service mode for running as daemon
    Service,
}

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

    /// Update satellite information
    pub fn update_satellite(&mut self, satellite: SatelliteInfo) {
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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            gps_data: GpsData::new(),
            mqtt_status: MqttStatus::Disconnected,
            serial_connected: false,
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Telemetry metrics calculated from GPS data
#[derive(Debug, Clone, Default)]
pub struct TelemetryMetrics {
    pub longitudinal_accel: Option<f64>, // m/s² (positive = acceleration, negative = braking)
    pub lateral_accel: Option<f64>,      // m/s² (lateral g-force)
    pub combined_g: Option<f64>,         // Total g-force magnitude
    pub heading_rate: Option<f64>,       // degrees/second
    pub distance_traveled: f64,          // Total distance in meters
    pub max_speed_kph: Option<f64>,      // Maximum speed recorded
    pub is_braking: bool,                // True if braking detected
}

/// Lap timing data
#[derive(Debug, Clone)]
pub struct LapData {
    pub lap_number: u32,
    pub lap_time_ms: Option<u64>,        // Current/last lap time in milliseconds
    pub best_lap_ms: Option<u64>,        // Best lap time in milliseconds
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
    Manual,     // Manually configured start/finish
    Learn,      // Learning mode - recording track
    Gpx,        // Loaded from GPX file
    Disabled,   // Lap timing disabled
}

/// A point on the track with geofence radius
#[derive(Debug, Clone)]
pub struct TrackPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: f64, // Geofence radius
    #[allow(dead_code)]
    pub name: String,       // e.g., "Start/Finish", "Sector 1", etc.
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
