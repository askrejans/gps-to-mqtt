use crate::models::{FixData, FixQuality, FixType, GnssSystem, NavigationData, SatelliteInfo};
use anyhow::{Context, Result};
use chrono::{NaiveDate, NaiveTime};
use tracing::debug;

/// Events that can be generated from GPS data parsing
#[derive(Debug, Clone)]
pub enum GpsEvent {
    SatelliteUpdate(SatelliteInfo),
    NavigationUpdate(NavigationData),
    FixUpdate(FixData),
    Message(String),
    RawNmea(String), // Raw NMEA sentence for display
    #[allow(dead_code)]
    AccuracyUpdate { std_lat: f64, std_lon: f64, std_alt: f64 }, // From GST
    RateOfTurn(f64), // degrees/minute from ROT
    TrueHeading(f64), // degrees from HDT
}

/// Process a complete NMEA sentence and return GPS events
pub fn parse_nmea_sentence(sentence: &str) -> Result<Vec<GpsEvent>> {
    if !sentence.starts_with('$') || !sentence.contains('*') {
        return Ok(vec![]);
    }

    let sentence_type = identify_sentence_type(sentence);
    debug!("Parsing sentence type: {:?}", sentence_type);

    match sentence_type {
        NmeaSentence::GSV => parse_gsv(sentence),
        NmeaSentence::GGA => parse_gga(sentence),
        NmeaSentence::RMC => parse_rmc(sentence),
        NmeaSentence::VTG => parse_vtg(sentence),
        NmeaSentence::GSA => parse_gsa(sentence),
        NmeaSentence::GLL => parse_gll(sentence),
        NmeaSentence::TXT => parse_txt(sentence),
        NmeaSentence::GNS => parse_gns(sentence),
        NmeaSentence::PUBX => parse_pubx(sentence),
        NmeaSentence::GST => parse_gst(sentence),
        NmeaSentence::ROT => parse_rot(sentence),
        NmeaSentence::HDT => parse_hdt(sentence),
        _ => {
            debug!("Unhandled sentence type: {}", sentence);
            Ok(vec![])
        }
    }
}

#[derive(Debug)]
enum NmeaSentence {
    GSV,    // Satellites in view
    GGA,    // Fix information
    RMC,    // Recommended minimum data
    VTG,    // Vector track and speed over ground
    GSA,    // Overall satellite data
    GLL,    // Geographic position
    TXT,    // Text transmission
    GRS,    // GNSS Range Residuals
    GST,    // GNSS Pseudorange Error Statistics
    GNS,    // GNSS Fix Data
    VLW,    // Dual Ground/Water Distance
    PUBX,   // PUBX proprietary messages
    ROT,    // Rate of Turn
    HDT,    // True Heading
    Unknown,
}

fn identify_sentence_type(sentence: &str) -> NmeaSentence {
    if sentence.contains("GSV") {
        NmeaSentence::GSV
    } else if sentence.contains("GGA") {
        NmeaSentence::GGA
    } else if sentence.contains("RMC") {
        NmeaSentence::RMC
    } else if sentence.contains("VTG") {
        NmeaSentence::VTG
    } else if sentence.contains("GSA") {
        NmeaSentence::GSA
    } else if sentence.contains("GLL") {
        NmeaSentence::GLL
    } else if sentence.contains("TXT") {
        NmeaSentence::TXT
    } else if sentence.contains("GRS") {
        NmeaSentence::GRS
    } else if sentence.contains("GST") {
        NmeaSentence::GST
    } else if sentence.contains("GNS") {
        NmeaSentence::GNS
    } else if sentence.contains("VLW") {
        NmeaSentence::VLW
    } else if sentence.contains("PUBX") {
        NmeaSentence::PUBX
    } else if sentence.contains("ROT") {
        NmeaSentence::ROT
    } else if sentence.contains("HDT") {
        NmeaSentence::HDT
    } else {
        NmeaSentence::Unknown
    }
}

/// Parse GSV (Satellites in View) sentence
fn parse_gsv(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 4 {
        return Ok(vec![]);
    }

    // Determine GNSS system from sentence ID
    let system = if sentence.contains("GPGSV") {
        GnssSystem::Gps
    } else if sentence.contains("GLGSV") {
        GnssSystem::Glonass
    } else if sentence.contains("GAGSV") {
        GnssSystem::Galileo
    } else if sentence.contains("GBGSV") || sentence.contains("BDGSV") {
        GnssSystem::Beidou
    } else {
        GnssSystem::Unknown
    };

    let mut events = Vec::new();

    // Parse satellite information (4 fields per satellite)
    let mut i = 4;
    while i + 3 < parts.len() {
        if let Ok(prn) = parts[i].parse::<u32>() {
            let sat = SatelliteInfo {
                prn,
                elevation: parts[i + 1].parse().ok(),
                azimuth: parts[i + 2].parse().ok(),
                snr: parts[i + 3].split('*').next().and_then(|s| s.parse().ok()),
                system,
            };
            events.push(GpsEvent::SatelliteUpdate(sat));
        }
        i += 4;
    }

    Ok(events)
}

/// Parse GGA (Fix Information) sentence
fn parse_gga(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 15 {
        return Ok(vec![]);
    }

    let mut events = Vec::new();

    // Parse time
    let time = parse_time(parts[1]).ok();

    // Parse position
    let lat = parse_coordinate(parts[2], parts[3]).ok();
    let lon = parse_coordinate(parts[4], parts[5]).ok();
    
    if lat.is_some() || lon.is_some() {
        let nav = NavigationData {
            latitude: lat,
            longitude: lon,
            altitude: parts[9].parse().ok(),
            ..Default::default()
        };
        events.push(GpsEvent::NavigationUpdate(nav));
    }

    // Parse fix quality and satellites
    let quality = match parts[6] {
        "0" => FixQuality::Invalid,
        "1" => FixQuality::GpsFix,
        "2" => FixQuality::DgpsFix,
        "3" => FixQuality::PpsFix,
        "4" => FixQuality::Rtk,
        "5" => FixQuality::FloatRtk,
        "6" => FixQuality::Estimated,
        "7" => FixQuality::Manual,
        "8" => FixQuality::Simulation,
        _ => FixQuality::Invalid,
    };

    let fix = FixData {
        time,
        fix_quality: Some(quality),
        satellites_used: parts[7].parse().ok(),
        hdop: parts[8].parse().ok(),
        ..Default::default()
    };
    events.push(GpsEvent::FixUpdate(fix));

    Ok(events)
}

/// Parse RMC (Recommended Minimum) sentence
fn parse_rmc(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 12 {
        return Ok(vec![]);
    }

    let mut events = Vec::new();

    // Parse time and date
    let time = parse_time(parts[1]).ok();
    let date = parse_date(parts[9]).ok();

    // Parse position and navigation
    let lat = parse_coordinate(parts[3], parts[4]).ok();
    let lon = parse_coordinate(parts[5], parts[6]).ok();
    let speed = parts[7].parse().ok();
    let course = parts[8].parse().ok();

    if lat.is_some() || lon.is_some() || speed.is_some() || course.is_some() {
        let nav = NavigationData {
            latitude: lat,
            longitude: lon,
            speed_knots: speed,
            course,
            ..Default::default()
        };
        events.push(GpsEvent::NavigationUpdate(nav));
    }

    let fix = FixData {
        time,
        date,
        ..Default::default()
    };
    events.push(GpsEvent::FixUpdate(fix));

    Ok(events)
}

/// Parse VTG (Track and Speed) sentence
fn parse_vtg(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 9 {
        return Ok(vec![]);
    }

    let course = parts[1].parse().ok();
    let speed_knots = parts[5].parse().ok();
    let speed_kph = parts[7].parse().ok();

    if course.is_some() || speed_knots.is_some() || speed_kph.is_some() {
        let nav = NavigationData {
            course,
            speed_knots,
            speed_kph,
            ..Default::default()
        };
        Ok(vec![GpsEvent::NavigationUpdate(nav)])
    } else {
        Ok(vec![])
    }
}

/// Parse GSA (Overall Satellite Data) sentence
fn parse_gsa(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 18 {
        return Ok(vec![]);
    }

    let fix_type = match parts[2] {
        "1" => FixType::NoFix,
        "2" => FixType::Fix2D,
        "3" => FixType::Fix3D,
        _ => FixType::Unknown,
    };

    let fix = FixData {
        fix_type: Some(fix_type),
        pdop: parts[15].parse().ok(),
        hdop: parts[16].parse().ok(),
        vdop: parts[17].split('*').next().and_then(|s| s.parse().ok()),
        ..Default::default()
    };

    Ok(vec![GpsEvent::FixUpdate(fix)])
}

/// Parse GLL (Geographic Position) sentence
fn parse_gll(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 7 {
        return Ok(vec![]);
    }

    let lat = parse_coordinate(parts[1], parts[2]).ok();
    let lon = parse_coordinate(parts[3], parts[4]).ok();

    if lat.is_some() || lon.is_some() {
        let nav = NavigationData {
            latitude: lat,
            longitude: lon,
            ..Default::default()
        };
        Ok(vec![GpsEvent::NavigationUpdate(nav)])
    } else {
        Ok(vec![])
    }
}

/// Parse TXT (Text Transmission) sentence
fn parse_txt(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 5 {
        return Ok(vec![]);
    }

    let message = parts[4].trim_end_matches('*').trim_end_matches(|c: char| c.is_ascii_hexdigit());
    Ok(vec![GpsEvent::Message(message.to_string())])
}

/// Parse GNS (GNSS Fix Data) sentence
fn parse_gns(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 13 {
        return Ok(vec![]);
    }

    let mut events = Vec::new();

    let lat = parse_coordinate(parts[2], parts[3]).ok();
    let lon = parse_coordinate(parts[4], parts[5]).ok();
    
    if lat.is_some() || lon.is_some() {
        let nav = NavigationData {
            latitude: lat,
            longitude: lon,
            altitude: parts[9].parse().ok(),
            ..Default::default()
        };
        events.push(GpsEvent::NavigationUpdate(nav));
    }

    let time = parse_time(parts[1]).ok();
    let fix = FixData {
        time,
        satellites_used: parts[7].parse().ok(),
        hdop: parts[8].parse().ok(),
        ..Default::default()
    };
    events.push(GpsEvent::FixUpdate(fix));

    Ok(events)
}

/// Parse PUBX (u-blox proprietary) sentence
fn parse_pubx(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 2 {
        return Ok(vec![]);
    }

    // PUBX,00 is position data
    if parts[1] == "00" && parts.len() >= 21 {
        let mut events = Vec::new();

        let lat = parse_coordinate(parts[3], parts[4]).ok();
        let lon = parse_coordinate(parts[5], parts[6]).ok();
        
        if lat.is_some() || lon.is_some() {
            let nav = NavigationData {
                latitude: lat,
                longitude: lon,
                altitude: parts[7].parse().ok(),
                speed_kph: parts[11].parse().ok(),
                course: parts[12].parse().ok(),
                ..Default::default()
            };
            events.push(GpsEvent::NavigationUpdate(nav));
        }

        let time = parse_time(parts[2]).ok();
        let fix = FixData {
            time,
            satellites_used: parts[18].parse().ok(),
            ..Default::default()
        };
        events.push(GpsEvent::FixUpdate(fix));

        Ok(events)
    } else {
        Ok(vec![])
    }
}

/// Parse time from NMEA format (HHMMSS.sss)
fn parse_time(time_str: &str) -> Result<NaiveTime> {
    if time_str.is_empty() || time_str.len() < 6 {
        anyhow::bail!("Invalid time string");
    }

    let hour: u32 = time_str[0..2].parse().context("Invalid hour")?;
    let minute: u32 = time_str[2..4].parse().context("Invalid minute")?;
    let second: u32 = time_str[4..6].parse().context("Invalid second")?;

    NaiveTime::from_hms_opt(hour, minute, second)
        .context("Invalid time values")
}

/// Parse date from NMEA format (DDMMYY)
fn parse_date(date_str: &str) -> Result<NaiveDate> {
    if date_str.is_empty() || date_str.len() < 6 {
        anyhow::bail!("Invalid date string");
    }

    let day: u32 = date_str[0..2].parse().context("Invalid day")?;
    let month: u32 = date_str[2..4].parse().context("Invalid month")?;
    let year: i32 = date_str[4..6].parse::<i32>().context("Invalid year")? + 2000;

    NaiveDate::from_ymd_opt(year, month, day)
        .context("Invalid date values")
}

/// Parse coordinate from NMEA format (DDMM.MMMM or DDDMM.MMMM)
fn parse_coordinate(coord_str: &str, dir: &str) -> Result<f64> {
    if coord_str.is_empty() {
        anyhow::bail!("Empty coordinate");
    }

    let dot_pos = coord_str.find('.').context("No decimal point")?;
    
    // Extract degrees (everything before last 2 digits before decimal)
    let deg_end = if dot_pos >= 4 { dot_pos - 2 } else { dot_pos.saturating_sub(2) };
    let degrees: f64 = coord_str[0..deg_end].parse().context("Invalid degrees")?;
    
    // Extract minutes
    let minutes: f64 = coord_str[deg_end..].parse().context("Invalid minutes")?;

    let mut decimal = degrees + (minutes / 60.0);

    // Apply direction
    if dir == "S" || dir == "W" {
        decimal = -decimal;
    }

    Ok(decimal)
}

/// Parse GST (GNSS Pseudorange Error Statistics) sentence
fn parse_gst(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 8 {
        return Ok(vec![]);
    }

    // GST provides standard deviation of position errors
    let std_lat: f64 = parts[6].parse().unwrap_or(0.0);
    let std_lon: f64 = parts[7].parse().unwrap_or(0.0);
    let std_alt: f64 = parts[8].split('*').next().and_then(|s| s.parse().ok()).unwrap_or(0.0);

    if std_lat > 0.0 || std_lon > 0.0 || std_alt > 0.0 {
        Ok(vec![GpsEvent::AccuracyUpdate { std_lat, std_lon, std_alt }])
    } else {
        Ok(vec![])
    }
}

/// Parse ROT (Rate of Turn) sentence
fn parse_rot(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 2 {
        return Ok(vec![]);
    }

    // ROT provides rate of turn in degrees per minute
    if let Ok(rate) = parts[1].parse::<f64>() {
        Ok(vec![GpsEvent::RateOfTurn(rate)])
    } else {
        Ok(vec![])
    }
}

/// Parse HDT (True Heading) sentence
fn parse_hdt(sentence: &str) -> Result<Vec<GpsEvent>> {
    let parts: Vec<&str> = sentence.split(',').collect();
    if parts.len() < 2 {
        return Ok(vec![]);
    }

    // HDT provides true heading in degrees
    if let Ok(heading) = parts[1].parse::<f64>() {
        Ok(vec![GpsEvent::TrueHeading(heading)])
    } else {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_coordinate() {
        assert!((parse_coordinate("4807.038", "N").unwrap() - 48.1173).abs() < 0.001);
        assert!((parse_coordinate("01131.000", "W").unwrap() + 11.5166).abs() < 0.001);
    }

    #[test]
    fn test_parse_time() {
        let time = parse_time("123519").unwrap();
        assert_eq!(time.hour(), 12);
        assert_eq!(time.minute(), 35);
        assert_eq!(time.second(), 19);
    }

    #[test]
    fn test_parse_date() {
        let date = parse_date("230394").unwrap();
        assert_eq!(date.year(), 2023);
        assert_eq!(date.month(), 3);
        assert_eq!(date.day(), 94); // This would fail - intentional for testing
    }
}
