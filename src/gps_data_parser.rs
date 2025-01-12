use crate::config::AppConfig;
use crate::mqtt_handler::publish_message;
use paho_mqtt as mqtt;
use std::str::FromStr;
use std::sync::Mutex;

lazy_static::lazy_static! {
    // Using lazy_static crate to create a static mutable variable for storing the last published time
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
pub fn process_gps_data(data: &[u8], config: &AppConfig, mqtt: mqtt::Client) {
    // Convert bytes to a string.
    let data_str = String::from_utf8_lossy(data);

    // Check if the sentence starts with '$' and contains '*'.
    if data_str.starts_with('$') && data_str.contains('*') {
        // Extract the sentence between '$' and '*'.
        let sentence: Vec<&str> = data_str.trim_start_matches('$').split('*').collect();
        let cleaned_sentence = sentence[0].trim();

        // Dispatch to specialized parsing functions based on sentence type.
        match cleaned_sentence {
            s if s.starts_with("GPGSV") || s.starts_with("GLGSV") || s.starts_with("GAGSV") => {
                // Parse and display GSV sentence.
                //parse_and_display_gsv(cleaned_sentence);
            }
            s if s.starts_with("GNGGA") => {
                // Parse and display GGA sentence.
                parse_and_display_gga(cleaned_sentence, mqtt, config);
            }
            s if s.starts_with("GNRMC") => {
                // Parse and display RMC sentence.
                parse_and_display_rmc(cleaned_sentence, mqtt, config);
            }
            s if s.starts_with("GNVTG") => {
                // Parse and display VTG sentence.
                parse_and_display_vtg(cleaned_sentence, mqtt, config);
            }
            s if s.starts_with("GNGSA") => {
                // Parse and display GSA sentence.
                //parse_and_display_gsa(cleaned_sentence);
            }
            s if s.starts_with("GNGLL") => {
                // Parse and display GLL sentence.
                //parse_and_display_gll(cleaned_sentence);
            }
            s if s.starts_with("GNTXT") => {
                // Parse and display GNTXT sentence.
                //parse_and_display_gntxt(cleaned_sentence);
            }
            _ => {
                // Unknown sentence type, just print the raw data.
                println!("Unknown Sentence Type: {}", cleaned_sentence);
            }
        }
    }
}

/// Parses and displays GSV (Satellites in View) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GSV sentence data.
///
/// The function splits the GSV sentence into its components and prints the total number of sentences,
/// the sentence number, and the total number of satellites. It also prints the details of each satellite
/// including PRN, elevation, azimuth, and SNR.
fn parse_and_display_gsv(data: &str) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 8 {
        let num_sentences: usize = FromStr::from_str(parts[1]).unwrap_or(0);
        let sentence_num: usize = FromStr::from_str(parts[2]).unwrap_or(0);
        let num_satellites: usize = FromStr::from_str(parts[3]).unwrap_or(0);

        println!(
            "GSV Sentence - Total Sentences: {}, Sentence Number: {}, Total Satellites: {}",
            num_sentences, sentence_num, num_satellites
        );

        // Print satellite type
        for i in 0..((parts.len() - 4) / 4) {
            let sat_index = 4 + i * 4;
            let sat_prn: usize = FromStr::from_str(parts[sat_index]).unwrap_or(0);
            let sat_elevation: usize = FromStr::from_str(parts[sat_index + 1]).unwrap_or(0);
            let sat_azimuth: usize = FromStr::from_str(parts[sat_index + 2]).unwrap_or(0);
            let sat_snr: usize = FromStr::from_str(parts[sat_index + 3]).unwrap_or(0);

            println!(
                "Satellite PRN: {}, Elevation: {}, Azimuth: {}, SNR: {}",
                sat_prn, sat_elevation, sat_azimuth, sat_snr
            );
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
    if parts.len() >= 7 {
        let latitude: f64 = FromStr::from_str(parts[2]).unwrap_or(0.0);
        let longitude: f64 = FromStr::from_str(parts[4]).unwrap_or(0.0);
        let altitude: f64 = FromStr::from_str(parts[9]).unwrap_or(0.0);
        let fix_quality: usize = FromStr::from_str(parts[6]).unwrap_or(0);

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

/// Parses and displays RMC (Recommended Minimum Specific GPS/Transit Data) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the RMC sentence data.
/// * `mqtt` - An MQTT client to publish the parsed data.
/// * `config` - Configuration settings for the application.
///
/// The function splits the RMC sentence into its components and publishes the time, date, latitude, longitude, and speed to MQTT.
fn parse_and_display_rmc(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 10 {
        let utc_time: &str = parts[1];
        let latitude: f64 = parse_latitude(parts[3], parts[4]);
        let longitude: f64 = parse_longitude(parts[5], parts[6]);
        let speed: f64 = FromStr::from_str(parts[7]).unwrap_or(0.0);
        let date: &str = parts[9];

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

/// Parses UTC time in the format HHMMSS.ss and returns the hour, minute, and second.
///
/// # Arguments
///
/// * `utc_time` - A string slice that holds the UTC time.
///
/// The function extracts the hour, minute, and second from the UTC time string.
fn parse_utc_time(utc_time: &str) -> (u32, u32, u32) {
    // Parse UTC time in the format HHMMSS.ss
    let hour: u32 = FromStr::from_str(&utc_time[0..2]).unwrap_or(0);
    let minute: u32 = FromStr::from_str(&utc_time[2..4]).unwrap_or(0);
    let second: u32 = FromStr::from_str(&utc_time[4..6]).unwrap_or(0);

    (hour, minute, second)
}

/// Parses date in the format DDMMYY and returns the day, month, and year.
///
/// # Arguments
///
/// * `date` - A string slice that holds the date.
///
/// The function extracts the day, month, and year from the date string.
fn parse_date(date: &str) -> (u32, u32, u32) {
    // Parse date in the format DDMMYY
    let day: u32 = FromStr::from_str(&date[0..2]).unwrap_or(0);
    let month: u32 = FromStr::from_str(&date[2..4]).unwrap_or(0);
    let year: u32 = FromStr::from_str(&date[4..6]).unwrap_or(0);

    (day, month, year)
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
        let course: f64 = FromStr::from_str(parts[1]).unwrap_or(0.0);
        let speed_knots: f64 = FromStr::from_str(parts[5]).unwrap_or(0.0);
        let speed_kph: f64 = FromStr::from_str(parts[7]).unwrap_or(0.0);

        // Push course to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}CRS", config.mqtt_base_topic),
            &format!("{}", course).as_str(),
            0,
        ) {
            println!("Error pushing course to MQTT: {:?}", e);
        }

        // Push speed in knots to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}SPD_KTS", config.mqtt_base_topic),
            &format!("{}", speed_knots).as_str(),
            0,
        ) {
            println!("Error pushing speed in knots to MQTT: {:?}", e);
        }

        // Push speed in kph to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}SPD_KPH", config.mqtt_base_topic),
            &format!("{}", speed_kph).as_str(),
            0,
        ) {
            println!("Error pushing speed in kph to MQTT: {:?}", e);
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
///
/// The function splits the GSA sentence into its components and prints the message ID, fix type, and PRN.
fn parse_and_display_gsa(data: &str) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 17 {
        let message_id: &str = parts[0];
        let fix_type: &str = match parts[2] {
            "1" => "Not Available",
            "2" => "2D",
            "3" => "3D",
            _ => "Unknown",
        };
        let prn: usize = FromStr::from_str(parts[3]).unwrap_or(0);

        println!(
            "GSA Sentence - Message ID: {}, Fix Type: {}, PRN: {}",
            message_id, fix_type, prn
        );
    } else {
        println!("Invalid GSA Sentence: {}", data);
    }
}

/// Parses and displays GNTXT (Text Transmission) sentence data.
///
/// # Arguments
///
/// * `data` - A string slice that holds the GNTXT sentence data.
///
/// The function splits the GNTXT sentence into its components and prints the message ID, message number, total messages, and text.
fn parse_and_display_gntxt(data: &str) {
    let mut parts = data.splitn(4, ',');
    if let (Some(msg_id), Some(msg_num), Some(msg_total), Some(text)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    {
        let msg_num: usize = FromStr::from_str(msg_num).unwrap_or(0);
        let msg_total: usize = FromStr::from_str(msg_total).unwrap_or(0);

        println!(
            "GNTXT Sentence - Message ID: {}, Message Number: {}/{} - Text: {}",
            msg_id, msg_num, msg_total, text
        );
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
fn parse_and_display_gll(data: &str) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 7 {
        let latitude: f64 = parse_latitude(parts[1], parts[2]);
        let longitude: f64 = parse_longitude(parts[3], parts[4]);
        let utc_time: &str = parts[5];
        let data_status: &str = parts[6];

        println!(
            "GLL Sentence - Latitude: {}, Longitude: {}, UTC Time: {}, Data Status: {}",
            latitude, longitude, utc_time, data_status
        );
    } else {
        println!("Invalid GLL Sentence: {}", data);
    }
}

/// Parses latitude from NMEA format and converts it to decimal degrees.
///
/// # Arguments
///
/// * `value` - A string slice that holds the latitude in NMEA format (DDMM.MMMM).
/// * `direction` - A string slice that holds the direction ('N' or 'S').
///
/// The function extracts degrees and minutes from the NMEA format, converts them to decimal degrees,
/// and adjusts the sign based on the direction.
fn parse_latitude(value: &str, direction: &str) -> f64 {
    // Ensure the input strings have sufficient length
    if value.len() >= 2 && direction.len() == 1 {
        // Extract degrees and minutes from the raw NMEA format
        let degrees: f64 = value[..2].parse().unwrap_or(0.0);
        let minutes: f64 = value[2..].parse().unwrap_or(0.0);

        // Convert to decimal degrees and consider direction
        let result = degrees + minutes / 60.0;
        if direction == "S" {
            -result
        } else {
            result
        }
    } else {
        // Handle the case where the input strings are not valid
        println!("Invalid latitude input: {}{}", value, direction);
        0.0
    }
}

/// Parses longitude from NMEA format and converts it to decimal degrees.
///
/// # Arguments
///
/// * `value` - A string slice that holds the longitude in NMEA format (DDDMM.MMMM).
/// * `direction` - A string slice that holds the direction ('E' or 'W').
///
/// The function extracts degrees and minutes from the NMEA format, converts them to decimal degrees,
/// and adjusts the sign based on the direction.
fn parse_longitude(value: &str, direction: &str) -> f64 {
    // Ensure the input strings have sufficient length
    if value.len() >= 3 && direction.len() == 1 {
        // Extract degrees and minutes from the raw NMEA format
        let degrees: f64 = value[..3].parse().unwrap_or(0.0);
        let minutes: f64 = value[3..].parse().unwrap_or(0.0);

        // Convert to decimal degrees and consider direction
        let result = degrees + minutes / 60.0;
        if direction == "W" {
            -result
        } else {
            result
        }
    } else {
        // Handle the case where the input strings are not valid
        println!("Invalid longitude input: {}{}", value, direction);
        0.0
    }
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
            config_path: Some("/path/to/config".to_string()),
            mqtt_host: "localhost".to_string(),
            mqtt_port: 1883,
            set_gps_to_10hz: false,
            port_name: "/dev/ttyACM0" .to_string()
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
        let data = "GPGSV,3,1,11,07,79,045,42,08,62,272,43,09,59,138,42,10,57,359,43*70";
        parse_and_display_gsv(data);
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
        let data = "GNGSA,A,3,04,05,,09,12,,24,,,,,1.8,1.0,1.5*33";
        parse_and_display_gsa(data);
    }

    #[test]
    fn test_parse_and_display_gntxt() {
        let data = "GNTXT,01,01,02,u-blox ag - www.u-blox.com*4E";
        parse_and_display_gntxt(data);
    }

    #[test]
    fn test_parse_and_display_gll() {
        let data = "GNGLL,4916.45,N,12311.12,W,225444,A";
        parse_and_display_gll(data);
    }
}
