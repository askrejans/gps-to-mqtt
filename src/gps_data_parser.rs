use crate::config::AppConfig;
use crate::mqtt_handler::publish_message;
use paho_mqtt as mqtt;
use std::error::Error;
use std::sync::Mutex;

#[derive(Debug)]
pub enum NmeaSentence {
    GSV, // Satellites in view
    GGA, // Fix information
    RMC, // Recommended minimum data
    VTG, // Vector track and speed over ground
    GSA, // Overall satellite data
    GLL, // Geographic position
    TXT, // Text transmission
    Unknown,
}

impl NmeaSentence {
    fn from_str(s: &str) -> Self {
        match s {
            s if s.contains("GSV") => NmeaSentence::GSV,
            s if s.contains("GGA") => NmeaSentence::GGA,
            s if s.contains("RMC") => NmeaSentence::RMC,
            s if s.contains("VTG") => NmeaSentence::VTG,
            s if s.contains("GSA") => NmeaSentence::GSA,
            s if s.contains("GLL") => NmeaSentence::GLL,
            s if s.contains("TXT") => NmeaSentence::TXT,
            _ => NmeaSentence::Unknown,
        }
    }
}

#[derive(Debug)]
enum SatelliteType {
    GPS,
    GLONASS,
    Galileo,
    BeiDou,
    Unknown,
}

impl SatelliteType {
    fn as_str(&self) -> &'static str {
        match self {
            SatelliteType::GPS => "GPS",
            SatelliteType::GLONASS => "GLONASS",
            SatelliteType::Galileo => "Galileo",
            SatelliteType::BeiDou => "BeiDou",
            SatelliteType::Unknown => "Unknown",
        }
    }
}

lazy_static::lazy_static! {
    static ref LAST_PUBLISHED_TIME: Mutex<Option<String>> = Mutex::new(None);
    static ref LAST_PUBLISHED_DATE: Mutex<Option<String>> = Mutex::new(None);
}

/// Process and print the received GPS data from NMEA-0183 messages.
///
/// This function takes a slice of bytes representing received data, converts it to a string,
/// extracts relevant sentences starting with '$' and containing '*', and dispatches them
/// to specialized parsing functions based on sentence type.
///
/// # Arguments
///
/// * `data` - A slice of bytes representing received data.
pub fn process_gps_data(
    data: &[u8],
    config: &AppConfig,
    mqtt: mqtt::Client,
) -> Result<(), Box<dyn Error>> {
    let data_str = String::from_utf8_lossy(data);

    // Early return if invalid format
    if !data_str.starts_with('$') || !data_str.contains('*') {
        return Ok(());
    }

    // Extract sentence using more efficient string operations
    let sentence = match data_str.split('*').next() {
        Some(s) => &s[1..], // Skip the '$' character
        None => return Ok(()),
    };

    // Parse sentence type and dispatch to appropriate handler
    match NmeaSentence::from_str(sentence) {
        NmeaSentence::GSV => parse_and_display_gsv(sentence, mqtt.clone(), config),
        NmeaSentence::GGA => parse_and_display_gga(sentence, mqtt.clone(), config),
        NmeaSentence::RMC => parse_and_display_rmc(sentence, mqtt.clone(), config),
        NmeaSentence::VTG => parse_and_display_vtg(sentence, mqtt.clone(), config),
        NmeaSentence::GSA => parse_and_display_gsa(sentence, mqtt.clone(), config),
        NmeaSentence::GLL => parse_and_display_gll(sentence, mqtt.clone(), config),
        NmeaSentence::TXT => parse_and_display_gntxt(sentence, mqtt.clone(), config),
        NmeaSentence::Unknown => {
            println!("Unknown Sentence Type: {}", sentence);
        }
    }

    Ok(())
}

/// Parses and displays GSV (Satellites in View) sentence data and publishes it to MQTT.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GSV sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the GSV sentence into its components and prints the total number of sentences,
/// the sentence number, and the total number of satellites. It also prints the details of each satellite
/// including PRN, elevation, azimuth, and SNR, and publishes this information to MQTT.
fn parse_and_display_gsv(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    // Extract message type prefix (e.g., "GP" from "$GPGSV")
    let msg_type = data.get(0..2).unwrap_or("--");
    let sat_type = match msg_type {
        "GP" => SatelliteType::GPS,
        "GL" => SatelliteType::GLONASS,
        "GA" => SatelliteType::Galileo,
        "BD" => SatelliteType::BeiDou,
        _ => {
            println!("Unknown satellite type prefix: {}", msg_type);
            SatelliteType::Unknown
        }
    };

    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 8 {
        let num_satellites = parts[3].parse::<usize>().unwrap_or(0);
        println!("Total Satellites: {}", num_satellites);

        // Publish total satellites count
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}SAT/GLOBAL/NUM", config.mqtt_base_topic),
            &format!("{}", num_satellites).as_str(),
            0,
        ) {
            println!("Error pushing total number of satellites to MQTT: {:?}", e);
        }

        // Process each satellite
        for i in 0..((parts.len() - 4) / 4) {
            let sat_index = 4 + i * 4;
            let sat_prn = parts[sat_index].parse::<usize>().unwrap_or(0);
            let sat_elevation = parts[sat_index + 1].parse::<usize>().unwrap_or(0);
            let sat_azimuth = parts[sat_index + 2].parse::<usize>().unwrap_or(0);
            let sat_snr = parts[sat_index + 3].parse::<usize>().unwrap_or(0);
            let in_view = sat_snr > 0;

            println!(
                "Satellite PRN: {}, Type: {}, Elevation: {}, Azimuth: {}, SNR: {}, In View: {}",
                sat_prn,
                sat_type.as_str(),
                sat_elevation,
                sat_azimuth,
                sat_snr,
                in_view
            );

            // Keep original MQTT topic structure
            let sat_topic = format!("{}SAT/VEHICLES/{}", config.mqtt_base_topic, sat_prn);
            let sat_info = format!(
                "PRN: {}, Type: {}, Elevation: {}, Azimuth: {}, SNR: {}, In View: {}",
                sat_prn,
                sat_type.as_str(),
                sat_elevation,
                sat_azimuth,
                sat_snr,
                in_view
            );

            if let Err(e) = publish_message(&mqtt, &sat_topic, &sat_info, 0) {
                println!("Error pushing satellite info to MQTT: {:?}", e);
            }
        }
    } else {
        println!("Invalid GSV Sentence: {}", data);
    }
}

/// Parses and displays GGA (Global Positioning System Fix Data) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GGA sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the GGA sentence into its components and publishes the altitude and fix quality to MQTT.
fn parse_and_display_gga(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();

    if parts.len() >= 10 {
        let latitude = parts[2].parse::<f64>().unwrap_or(0.0);
        let longitude = parts[4].parse::<f64>().unwrap_or(0.0);
        let altitude = parts[9].parse::<f64>().unwrap_or(0.0);
        let fix_quality = parts[6].parse::<usize>().unwrap_or(0);

        println!("Latitude: {}", latitude);
        println!("Longitude: {}", longitude);
        println!("Altitude: {}", altitude);

        // Push altitude to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}ALT", config.mqtt_base_topic),
            &format!("{}", altitude).as_str(),
            0,
        ) {
            println!("Error pushing altitude to MQTT: {:?}", e);
        }

        // Push fix quality to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}QTY", config.mqtt_base_topic),
            &format!("{}", fix_quality).as_str(),
            0,
        ) {
            println!("Error pushing fix quality to MQTT: {:?}", e);
        }
    } else {
        println!("Invalid GGA Sentence: {}", data);
    }
}

/// Parses and displays RMC (Recommended Minimum Specific GNSS Data) sentence data and publishes it to MQTT.
///
/// # Arguments
///
/// * `data` - A string slice that holds the RMC sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the RMC sentence into its components, prints the latitude, longitude, UTC time, and data status,
/// and publishes the RMC time, latitude, longitude, and speed to MQTT.
fn parse_and_display_rmc(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 10 {
        let utc_time = parts[1];
        let latitude = parse_latitude(parts[3], parts[4]);
        let longitude = parse_longitude(parts[5], parts[6]);
        let speed = parts[7].parse::<f64>().unwrap_or(0.0);
        let date = parts[9];

        // Parse UTC time and date
        let (hour, minute, second) = parse_utc_time(utc_time);
        let (day, month, year) = parse_date(date);

        // Push time to MQTT
        let current_time = format!("{:02}:{:02}:{:02}", hour, minute, second);

        let mut last_published_time = LAST_PUBLISHED_TIME.lock().unwrap();
        if last_published_time.as_deref() != Some(&current_time) {
            if let Err(e) = publish_message(
                &mqtt,
                &format!("{}TME", config.mqtt_base_topic),
                &current_time,
                0,
            ) {
                println!("Error pushing time to MQTT: {:?}", e);
            }
            *last_published_time = Some(current_time);
        }

        // Push date to MQTT
        let current_date = format!("{:02}.{:02}.20{:02}", day, month, year);

        let mut last_published_date = LAST_PUBLISHED_DATE.lock().unwrap();
        if last_published_date.as_deref() != Some(&current_date) {
            if let Err(e) = publish_message(&mqtt, "/GOLF86/GPS/DTE", &current_date, 0) {
                println!("Error pushing date to MQTT: {:?}", e);
            }
            *last_published_date = Some(current_date);
        }

        // Push latitude to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}LAT", config.mqtt_base_topic),
            &format!("{}", latitude).as_str(),
            0,
        ) {
            println!("Error pushing latitude to MQTT: {:?}", e);
        }

        // Push longitude to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}LNG", config.mqtt_base_topic),
            &format!("{}", longitude).as_str(),
            0,
        ) {
            println!("Error pushing longitude to MQTT: {:?}", e);
        }

        // Push speed to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}SPD", config.mqtt_base_topic),
            &format!("{}", speed).as_str(),
            0,
        ) {
            println!("Error pushing speed to MQTT: {:?}", e);
        }
    } else {
        println!("Invalid RMC Sentence: {}", data);
    }
}

/// Parses and displays VTG (Course Over Ground and Ground Speed) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the VTG sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the VTG sentence into its components and publishes the course, speed in knots, and speed in kph to MQTT.
fn parse_and_display_vtg(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 9 {
        let course = parts[1].parse::<f64>().unwrap_or(0.0);
        let speed_knots = parts[5].parse::<f64>().unwrap_or(0.0);
        let speed_kph = parts[7].parse::<f64>().unwrap_or(0.0);

        let messages = [
            (course, "CRS"),
            (speed_knots, "SPD_KTS"),
            (speed_kph, "SPD_KPH"),
        ];

        for (value, suffix) in &messages {
            if let Err(e) = publish_message(
                &mqtt,
                &format!("{}{}", config.mqtt_base_topic, suffix),
                &format!("{}", value).as_str(),
                0,
            ) {
                println!("Error pushing {} to MQTT: {:?}", suffix, e);
            }
        }
    } else {
        println!("Invalid VTG Sentence: {}", data);
    }
}

/// Parses and displays GSA (GNSS DOP and Active Satellites) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GSA sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the GSA sentence into its components and prints the message ID, fix type, and PRN.
fn parse_and_display_gsa(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 17 {
        let message_id = parts[0];
        let fix_type = match parts[2] {
            "1" => "Not Available",
            "2" => "2D",
            "3" => "3D",
            _ => "Unknown",
        };
        let prn = parts[3].parse::<usize>().unwrap_or(0);

        println!(
            "GSA Sentence - Message ID: {}, Fix Type: {}, PRN: {}",
            message_id, fix_type, prn
        );

        // Publish fix type to MQTT
        let sat_topic = format!("{}SAT/VEHICLES/{}/FIX_TYPE", config.mqtt_base_topic, prn);
        if let Err(e) = publish_message(&mqtt, &sat_topic, fix_type, 0) {
            println!("Error pushing fix type to MQTT: {:?}", e);
        }
    } else {
        println!("Invalid GSA Sentence: {}", data);
    }
}

/// Parses and displays GNTXT (Text Transmission) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GNTXT sentence data.
/// * `mqtt` - An MQTT client used to publish messages.
/// * `config` - Configuration settings for the application.
///
/// The function splits the GNTXT sentence into its components and prints the message ID, message number, total messages, and text.
/// If the message contains "ANTSTATUS=", it publishes the value after "=" to the MQTT topic.
/// If the message contains "PF=", it publishes the value after "=" to the MQTT topic.
/// If the message contains "GNSS OTP=", it prints the value after "=".

fn parse_and_display_gntxt(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let mut parts = data.splitn(4, ',');
    if let (Some(_msg_id), Some(_msg_num), Some(_msg_total), Some(text)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    {
        let message = text.splitn(2, ',').nth(1).unwrap_or(text);

        if message.contains("txbuf alloc") {
            return;
        }

        println!("GNTXT Text: {}", message);

        let topics = [
            ("ANTSTATUS=", "SAT/GLOBAL/ANTSTATUS"),
            ("PF=", "SAT/GLOBAL/PF"),
            ("GNSS OTP=", "SAT/GLOBAL/GNSS_OTP"),
        ];

        for (prefix, topic_suffix) in &topics {
            if let Some(value) = message.strip_prefix(prefix) {
                if let Err(e) = publish_message(
                    &mqtt,
                    &format!("{}{}", config.mqtt_base_topic, topic_suffix),
                    value,
                    0,
                ) {
                    println!(
                        "Error pushing {} to MQTT: {:?}",
                        prefix.trim_end_matches('='),
                        e
                    );
                }
                break;
            }
        }
    } else {
        println!("Invalid GNTXT Sentence: {}", data);
    }
}

/// Parses and displays GLL (Geographic Position - Latitude/Longitude) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GLL sentence data.
///
/// The function splits the GLL sentence into its components and prints the latitude, longitude, UTC time, and data status.
fn parse_and_display_gll(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() < 7 {
        println!("Invalid GLL Sentence: {}", data);
        return;
    }

    let latitude = parse_latitude(parts[1], parts[2]);
    let longitude = parse_longitude(parts[3], parts[4]);
    let utc_time = parts[5];

    // Parse UTC time
    let (hour, minute, second) = parse_utc_time(utc_time);
    let current_time = format!("{:02}:{:02}:{:02}", hour, minute, second);

    println!(
        "GLL Latitude: {}, GLL Longitude: {}, GLL UTC Time: {}",
        latitude, longitude, current_time
    );

    // Helper function to publish messages to MQTT
    fn publish_gll_message(
        mqtt: &mqtt::Client,
        topic_suffix: &str,
        message: &str,
        config: &AppConfig,
    ) {
        if let Err(e) = publish_message(
            mqtt,
            &format!("{}{}", config.mqtt_base_topic, topic_suffix),
            message,
            0,
        ) {
            println!("Error pushing GLL {} to MQTT: {:?}", topic_suffix, e);
        }
    }

    // Push GLL data to MQTT
    publish_gll_message(&mqtt, "GLL_TME", &current_time, config);
    publish_gll_message(&mqtt, "GLL_LAT", &latitude.to_string(), config);
    publish_gll_message(&mqtt, "GLL_LNG", &longitude.to_string(), config);
}

/// Parses latitude or longitude from NMEA format and converts it to decimal degrees.
///
/// # Arguments
///
/// * `value` - A string slice that holds the coordinate in NMEA format.
/// * `direction` - A string slice that holds the direction ('N', 'S', 'E', or 'W').
/// * `degree_len` - The length of the degree part in the NMEA format (2 for latitude, 3 for longitude).
///
/// The function extracts degrees and minutes from the NMEA format, converts them to decimal degrees,
/// and adjusts the sign based on the direction.
fn parse_coordinate(value: &str, direction: &str, degree_len: usize) -> f64 {
    if value.is_empty() || direction.is_empty() {
        println!("Invalid coordinate input: {}{}", value, direction);
        return 0.0;
    }

    if value.len() <= degree_len {
        println!("Invalid coordinate input: {}{}", value, direction);
        return 0.0;
    }

    if !matches!(direction, "N" | "S" | "E" | "W") {
        println!("Invalid direction: {}", direction);
        return 0.0;
    }

    // Parse degrees and minutes
    match (
        value[..degree_len].parse::<f64>(),
        value[degree_len..].parse::<f64>(),
    ) {
        (Ok(degrees), Ok(minutes)) => {
            let result = degrees + minutes / 60.0;
            match direction {
                "S" | "W" => -result,
                _ => result,
            }
        }
        _ => {
            println!("Failed to parse coordinate: {}{}", value, direction);
            0.0
        }
    }
}

/// Parses latitude from NMEA format and converts it to decimal degrees.
fn parse_latitude(value: &str, direction: &str) -> f64 {
    parse_coordinate(value, direction, 2)
}

/// Parses longitude from NMEA format and converts it to decimal degrees.
fn parse_longitude(value: &str, direction: &str) -> f64 {
    parse_coordinate(value, direction, 3)
}

/// Parses UTC time from NMEA HHMMSS.ss format into hour, minute, second components.
///
/// # Arguments
///
/// * `utc_time` - A string slice in HHMMSS format (e.g., "235959" for 23:59:59)
///                Optionally may contain decimal seconds after period
///
/// # Returns
///
/// A tuple of `(hour, minute, second)` where:
/// * `hour` - Hours in 24-hour format (0-23)
/// * `minute` - Minutes (0-59)  
/// * `second` - Seconds (0-59)
///
/// Returns `(0, 0, 0)` if:
/// * Input string is less than 6 characters
/// * Any time component is out of valid range
/// * Any component fails to parse as a number
///
fn parse_utc_time(utc_time: &str) -> (u32, u32, u32) {
    if utc_time.len() < 6 {
        return (0, 0, 0);
    }

    let hour = utc_time[0..2].parse::<u32>().unwrap_or(0);
    let minute = utc_time[2..4].parse::<u32>().unwrap_or(0);
    let second = utc_time[4..6].parse::<u32>().unwrap_or(0);

    if hour > 23 || minute > 59 || second > 59 {
        return (0, 0, 0);
    }

    (hour, minute, second)
}

/// Parses a date string in DDMMYY format and returns the components as integers.
///
/// # Arguments
///
/// * `date` - A string slice in DDMMYY format (e.g., "230394" for March 23, 1994)
///
/// # Returns
///
/// A tuple of `(day, month, year)` where:
/// * `day` - Day of month (1-31)
/// * `month` - Month number (1-12)
/// * `year` - Two-digit year (0-99)
///
/// Returns `(0, 0, 0)` if:
/// * Input string is not exactly 6 characters
/// * Day is 0 or > 31
/// * Month is 0 or > 12
/// * Any component fails to parse as a number
///
fn parse_date(date: &str) -> (u32, u32, u32) {
    if date.len() != 6 {
        return (0, 0, 0);
    }

    let day = date[0..2].parse::<u32>().unwrap_or(0);
    let month = date[2..4].parse::<u32>().unwrap_or(0);
    let year = date[4..6].parse::<u32>().unwrap_or(0);

    if day == 0 || day > 31 || month == 0 || month > 12 {
        return (0, 0, 0);
    }

    (day, month, year)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use paho_mqtt as mqtt;

    fn get_test_config() -> AppConfig {
        AppConfig {
            mqtt_base_topic: "/GOLF86/GPS/".to_string(),
            baud_rate: 9600,
            mqtt_host: "localhost".to_string(),
            mqtt_port: 1883,
            set_gps_to_10hz: false,
            port_name: "/dev/ttyACM0".to_string(),
        }
    }

    #[test]
    fn test_parse_latitude() {
        assert_eq!(parse_latitude("4916.45", "N"), 49.274166666666666);
        assert_eq!(parse_latitude("4916.45", "S"), -49.274166666666666);
        assert_eq!(parse_latitude("0000.00", "N"), 0.0);
        assert_eq!(parse_latitude("0000.00", "S"), -0.0);
    }

    #[test]
    fn test_parse_longitude() {
        assert_eq!(parse_longitude("12311.12", "E"), 123.18533333333333);
        assert_eq!(parse_longitude("12311.12", "W"), -123.18533333333333);
        assert_eq!(parse_longitude("00000.00", "E"), 0.0);
        assert_eq!(parse_longitude("00000.00", "W"), -0.0);
    }

    #[test]
    fn test_parse_utc_time() {
        assert_eq!(parse_utc_time("123519"), (12, 35, 19));
        assert_eq!(parse_utc_time("000000"), (0, 0, 0));
        assert_eq!(parse_utc_time("235959"), (23, 59, 59));
    }

    #[test]
    fn test_parse_date() {
        assert_eq!(parse_date("230394"), (23, 3, 94));
        assert_eq!(parse_date("010100"), (1, 1, 0));
        assert_eq!(parse_date("311299"), (31, 12, 99));
    }

    #[test]
    fn test_parse_and_display_gsv() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GPGSV,3,1,11,07,79,045,42,08,62,272,43,09,59,138,42,10,57,359,43*70";
        parse_and_display_gsv(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_gga() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47";
        parse_and_display_gga(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_rmc() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A";
        parse_and_display_rmc(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_vtg() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNVTG,054.7,T,034.4,M,005.5,N,010.2,K*48";
        parse_and_display_vtg(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_gsa() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNGSA,A,3,04,05,,09,12,,24,,,,,1.8,1.0,1.5*33";
        parse_and_display_gsa(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_gntxt() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNTXT,01,01,02,u-blox ag - www.u-blox.com*4E";
        parse_and_display_gntxt(data, mqtt, &config);
    }

    #[test]
    fn test_parse_and_display_gll() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();
        let data = "GNGLL,4916.45,N,12311.12,W,225444,A";
        parse_and_display_gll(data, mqtt, &config);
    }

    #[test]
    fn test_nmea_sentence_from_str() {
        assert!(matches!(NmeaSentence::from_str("GPGSV"), NmeaSentence::GSV));
        assert!(matches!(NmeaSentence::from_str("GNGGA"), NmeaSentence::GGA));
        assert!(matches!(NmeaSentence::from_str("GNRMC"), NmeaSentence::RMC));
        assert!(matches!(NmeaSentence::from_str("GNVTG"), NmeaSentence::VTG));
        assert!(matches!(NmeaSentence::from_str("GNGSA"), NmeaSentence::GSA));
        assert!(matches!(NmeaSentence::from_str("GNGLL"), NmeaSentence::GLL));
        assert!(matches!(NmeaSentence::from_str("GNTXT"), NmeaSentence::TXT));
        assert!(matches!(
            NmeaSentence::from_str("INVALID"),
            NmeaSentence::Unknown
        ));
    }

    #[test]
    fn test_satellite_type_as_str() {
        assert_eq!(SatelliteType::GPS.as_str(), "GPS");
        assert_eq!(SatelliteType::GLONASS.as_str(), "GLONASS");
        assert_eq!(SatelliteType::Galileo.as_str(), "Galileo");
        assert_eq!(SatelliteType::BeiDou.as_str(), "BeiDou");
        assert_eq!(SatelliteType::Unknown.as_str(), "Unknown");
    }

    #[test]
    fn test_process_gps_data_invalid_input() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();

        // Test data not starting with $
        let result = process_gps_data(b"Invalid data", &config, mqtt.clone());
        assert!(result.is_ok());

        // Test data without checksum separator
        let result = process_gps_data(b"$GPGGA,Invalid", &config, mqtt.clone());
        assert!(result.is_ok());

        // Test empty data
        let result = process_gps_data(b"", &config, mqtt.clone());
        assert!(result.is_ok());
    }

    #[test]
    fn test_coordinate_parsing_edge_cases() {
        // Test empty inputs
        assert_eq!(parse_latitude("", "N"), 0.0);
        assert_eq!(parse_longitude("", "E"), 0.0);

        // Test invalid directions
        assert_eq!(parse_latitude("4916.45", "X"), 0.0);
        assert_eq!(parse_longitude("12311.12", "Y"), 0.0);

        // Test invalid number formats
        assert_eq!(parse_latitude("abc.de", "N"), 0.0);
        assert_eq!(parse_longitude("xyz.wq", "E"), 0.0);

        // Test valid boundary values
        assert_eq!(parse_latitude("9000.00", "N"), 90.0);
        assert_eq!(parse_longitude("18000.00", "E"), 180.0);

        // Test short inputs
        assert_eq!(parse_latitude("1", "N"), 0.0);
        assert_eq!(parse_longitude("1", "E"), 0.0);
    }

    #[test]
    fn test_time_parsing_edge_cases() {
        // Test empty string
        assert_eq!(parse_utc_time(""), (0, 0, 0));

        // Test invalid formats
        assert_eq!(parse_utc_time("abc"), (0, 0, 0));
        assert_eq!(parse_utc_time("12"), (0, 0, 0));

        // Test invalid values
        assert_eq!(parse_utc_time("246101"), (0, 0, 0));

        // Test valid values
        assert_eq!(parse_utc_time("235959"), (23, 59, 59));
        assert_eq!(parse_utc_time("000000"), (0, 0, 0));
    }

    #[test]
    fn test_date_parsing_edge_cases() {
        // Test empty string
        assert_eq!(parse_date(""), (0, 0, 0));

        // Test invalid formats
        assert_eq!(parse_date("abc"), (0, 0, 0));
        assert_eq!(parse_date("12"), (0, 0, 0));

        // Test invalid values
        assert_eq!(parse_date("326599"), (0, 0, 0));

        // Test valid values
        assert_eq!(parse_date("311299"), (31, 12, 99));
        assert_eq!(parse_date("010100"), (1, 1, 0));
    }

    #[test]
    fn test_gsa_parsing_invalid_input() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();

        // Test with empty data
        let data = "GNGSA,,,,,,,,,,,,,,,,,";
        parse_and_display_gsa(data, mqtt.clone(), &config);

        // Test with invalid fix type
        let data = "GNGSA,A,9,04,05,,09,12,,24,,,,,1.8,1.0,1.5*33";
        parse_and_display_gsa(data, mqtt, &config);
    }

    #[test]
    fn test_gll_parsing_invalid_input() {
        let config = get_test_config();
        let mqtt = mqtt::Client::new("tcp://localhost:1883").unwrap();

        // Test with insufficient fields
        let data = "GNGLL,4916.45,N,12311.12";
        parse_and_display_gll(data, mqtt.clone(), &config);

        // Test with invalid coordinates
        let data = "GNGLL,invalid,N,invalid,W,225444,A";
        parse_and_display_gll(data, mqtt, &config);
    }
}
