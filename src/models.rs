//! Canonical, receiver-independent GNSS data model.
//!
//! Units are SI at rest (metres, metres/second, degrees, seconds). Unit
//! conversions happen only at output boundaries.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::Serialize;

pub const MPS_TO_KPH: f64 = 3.6;
pub const MPS_TO_MPH: f64 = 2.236_936_292_054_402;
pub const MPS_TO_KNOTS: f64 = 1.943_844_492_440_604_7;
pub const KNOTS_TO_MPS: f64 = 1852.0 / 3600.0;
pub const M_TO_FT: f64 = 3.280_839_895;

/// GNSS constellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Constellation {
    Gps,
    Sbas,
    Glonass,
    Galileo,
    Beidou,
    Qzss,
    Navic,
    Unknown,
}

impl Constellation {
    /// Classify a satellite from its talker and PRN.
    ///
    /// Covers both the NMEA ≤4.0 extended numbering (everything under `GP`/`GN`
    /// with GLONASS at 65–96 etc.) and NMEA 4.10+ per-system numbering.
    pub fn classify(talker: &str, prn: u32) -> Self {
        match talker {
            "GL" => Self::Glonass,
            "GA" => Self::Galileo,
            "GB" | "BD" => Self::Beidou,
            "GQ" | "QZ" => Self::Qzss,
            "GI" => Self::Navic,
            _ => match prn {
                1..=32 => Self::Gps,
                33..=64 | 120..=158 => Self::Sbas,
                65..=96 => Self::Glonass,
                193..=202 => Self::Qzss,
                201..=263 | 401..=463 => Self::Beidou,
                301..=336 => Self::Galileo,
                _ => Self::Unknown,
            },
        }
    }

    /// NMEA 4.10 GNSS system ID (GSA field 18 / GSV last field).
    pub fn from_system_id(id: &str) -> Option<Self> {
        Some(match id {
            "1" => Self::Gps,
            "2" => Self::Glonass,
            "3" => Self::Galileo,
            "4" => Self::Beidou,
            "5" => Self::Qzss,
            "6" => Self::Navic,
            _ => return None,
        })
    }

    /// RINEX-style one-letter prefix.
    pub fn letter(self) -> char {
        match self {
            Self::Gps => 'G',
            Self::Sbas => 'S',
            Self::Glonass => 'R',
            Self::Galileo => 'E',
            Self::Beidou => 'C',
            Self::Qzss => 'J',
            Self::Navic => 'I',
            Self::Unknown => '?',
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Gps => "GPS",
            Self::Sbas => "SBAS",
            Self::Glonass => "GLONASS",
            Self::Galileo => "Galileo",
            Self::Beidou => "BeiDou",
            Self::Qzss => "QZSS",
            Self::Navic => "NavIC",
            Self::Unknown => "Other",
        }
    }

    /// gpsd `gnssid` numbering.
    pub fn gpsd_id(self) -> u8 {
        match self {
            Self::Gps | Self::Unknown => 0,
            Self::Sbas => 1,
            Self::Galileo => 2,
            Self::Beidou => 3,
            Self::Qzss => 5,
            Self::Glonass => 6,
            Self::Navic => 7,
        }
    }
}

/// Dimensional fix mode (GSA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum FixMode {
    #[default]
    #[serde(rename = "none")]
    NoFix,
    #[serde(rename = "2d")]
    Fix2D,
    #[serde(rename = "3d")]
    Fix3D,
}

impl FixMode {
    /// gpsd TPV `mode` (0 unknown, 1 no fix, 2 2D, 3 3D).
    pub fn gpsd(self) -> u8 {
        match self {
            Self::NoFix => 1,
            Self::Fix2D => 2,
            Self::Fix3D => 3,
        }
    }
}

/// GGA fix quality indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixQuality {
    #[default]
    Invalid,
    Gps,
    Dgps,
    Pps,
    Rtk,
    FloatRtk,
    Estimated,
    Manual,
    Simulation,
}

impl FixQuality {
    pub fn from_gga(field: &str) -> Self {
        match field {
            "1" => Self::Gps,
            "2" => Self::Dgps,
            "3" => Self::Pps,
            "4" => Self::Rtk,
            "5" => Self::FloatRtk,
            "6" => Self::Estimated,
            "7" => Self::Manual,
            "8" => Self::Simulation,
            _ => Self::Invalid,
        }
    }

    /// Mode indicator used by RMC/GNS/VTG/GLL (NMEA 2.3+).
    pub fn from_mode_char(c: char) -> Option<Self> {
        Some(match c {
            'A' => Self::Gps,
            'D' => Self::Dgps,
            'P' => Self::Pps,
            'R' => Self::Rtk,
            'F' => Self::FloatRtk,
            'E' => Self::Estimated,
            'M' => Self::Manual,
            'S' => Self::Simulation,
            'N' => Self::Invalid,
            _ => return None,
        })
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    /// True for fixes from real measurements (excludes dead reckoning/manual/simulation).
    pub fn is_measured(self) -> bool {
        matches!(
            self,
            Self::Gps | Self::Dgps | Self::Pps | Self::Rtk | Self::FloatRtk
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Invalid => "Invalid",
            Self::Gps => "GPS",
            Self::Dgps => "DGPS",
            Self::Pps => "PPS",
            Self::Rtk => "RTK fixed",
            Self::FloatRtk => "RTK float",
            Self::Estimated => "Dead reckoning",
            Self::Manual => "Manual",
            Self::Simulation => "Simulation",
        }
    }
}

/// Provenance of [`Fix::accuracy_m`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccuracySource {
    /// 1-σ horizontal error reported by the receiver (GST).
    Gst,
    /// Receiver-estimated horizontal accuracy (u-blox PUBX,00 `hAcc`).
    Receiver,
    /// HDOP × 5 m (minimum 3 m) — a labelled estimate, not a measurement.
    HdopEstimate,
}

/// One satellite as last reported by GSV.
#[derive(Debug, Clone, Serialize)]
pub struct Satellite {
    pub constellation: Constellation,
    pub prn: u32,
    pub elevation_deg: Option<i32>,
    pub azimuth_deg: Option<i32>,
    pub snr_dbhz: Option<i32>,
    pub used: bool,
}

impl Satellite {
    /// Stable identifier such as `G07`, `R65`, `E11`.
    pub fn id(&self) -> String {
        format!("{}{:02}", self.constellation.letter(), self.prn)
    }
}

/// Satellite sky view snapshot.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Sky {
    pub satellites: Vec<Satellite>,
    pub in_view: usize,
    pub used: usize,
    pub hdop: Option<f64>,
    pub vdop: Option<f64>,
    pub pdop: Option<f64>,
}

/// Racing / vehicle dynamics derived from consecutive fixes.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Telemetry {
    /// m/s² (+accelerating / −braking)
    pub longitudinal_accel_mps2: Option<f64>,
    pub longitudinal_g: Option<f64>,
    /// m/s² centripetal (v × ω)
    pub lateral_accel_mps2: Option<f64>,
    pub lateral_g: Option<f64>,
    pub combined_g: Option<f64>,
    /// deg/s yaw rate (ROT preferred, course difference fallback)
    pub yaw_rate_dps: Option<f64>,
    /// Session odometer, metres
    pub distance_m: f64,
    /// Session maximum speed, m/s
    pub max_speed_mps: f64,
    pub braking: bool,
}

/// One coherent navigation epoch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Fix {
    /// Monotonic counter of emitted fixes in this process.
    pub sequence: u64,
    /// Receiver UTC time (date + time); `None` until a date has been seen.
    pub time: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub utc_time: Option<NaiveTime>,
    #[serde(skip)]
    pub utc_date: Option<NaiveDate>,
    /// Host wall-clock time when the epoch was completed (Unix ms).
    pub received_ms: i64,
    /// True when the receiver reports a usable, measured position.
    pub valid: bool,
    pub mode: FixMode,
    pub quality: FixQuality,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Altitude above mean sea level, metres.
    pub altitude_m: Option<f64>,
    /// Geoid separation (ellipsoid − MSL), metres.
    pub geoid_separation_m: Option<f64>,
    pub speed_mps: Option<f64>,
    /// Course over ground, degrees true.
    pub course_deg: Option<f64>,
    /// True heading from a heading sensor (HDT/THS), degrees.
    pub heading_deg: Option<f64>,
    pub magnetic_variation_deg: Option<f64>,
    /// Rate of turn (ROT), deg/s.
    pub rate_of_turn_dps: Option<f64>,
    pub satellites_used: Option<u32>,
    pub satellites_in_view: Option<u32>,
    pub hdop: Option<f64>,
    pub vdop: Option<f64>,
    pub pdop: Option<f64>,
    /// Horizontal 1-σ accuracy, metres. See `accuracy_source`.
    pub accuracy_m: Option<f64>,
    pub accuracy_source: Option<AccuracySource>,
    /// GST per-axis 1-σ errors, metres.
    pub error_lat_m: Option<f64>,
    pub error_lon_m: Option<f64>,
    pub error_alt_m: Option<f64>,
    pub dgps_age_s: Option<f64>,
    pub dgps_station: Option<u32>,
    /// True when static hold replaced a stationary position with the held mean.
    pub position_held: bool,
    /// MGRS reference, 10 digits (1 m), e.g. `35V LC 31524 16761`.
    pub mgrs: Option<String>,
    /// MGRS reference, 8 digits (10 m), e.g. `35V LC 3152 1676`.
    pub mgrs_8: Option<String>,
    pub telemetry: Option<Telemetry>,
}

impl Fix {
    pub fn speed_kph(&self) -> Option<f64> {
        self.speed_mps.map(|v| v * MPS_TO_KPH)
    }

    /// Valid position as `(lat, lon)`.
    pub fn position(&self) -> Option<(f64, f64)> {
        match (self.valid, self.latitude, self.longitude) {
            (true, Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_constellations() {
        assert_eq!(Constellation::classify("GP", 7), Constellation::Gps);
        assert_eq!(Constellation::classify("GN", 70), Constellation::Glonass);
        assert_eq!(Constellation::classify("GP", 46), Constellation::Sbas);
        assert_eq!(Constellation::classify("GA", 7), Constellation::Galileo);
        assert_eq!(Constellation::classify("GB", 7), Constellation::Beidou);
        assert_eq!(Constellation::classify("GQ", 2), Constellation::Qzss);
        assert_eq!(Constellation::classify("GN", 999), Constellation::Unknown);
    }

    #[test]
    fn quality_codes_match_gga() {
        for (field, code) in [("0", 0), ("1", 1), ("4", 4), ("8", 8), ("x", 0)] {
            assert_eq!(FixQuality::from_gga(field).code(), code);
        }
        assert!(FixQuality::Rtk.is_measured());
        assert!(!FixQuality::Estimated.is_measured());
    }

    #[test]
    fn unit_constants() {
        assert!((25.0 * MPS_TO_KPH - 90.0).abs() < 1e-9);
        assert!((KNOTS_TO_MPS * MPS_TO_KNOTS - 1.0).abs() < 1e-12);
        assert!((100.0 / 3.6 * MPS_TO_MPH - 62.137).abs() < 0.001);
    }
}
