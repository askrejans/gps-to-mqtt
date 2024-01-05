use crate::config::AppConfig;
use crate::mqtt_handler::{publish_message, setup_mqtt};
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
pub fn process_gps_data(data: &[u8], config: &AppConfig) {
    let mqtt = setup_mqtt(&config);

    // Convert bytes to a string.
    let data_str = String::from_utf8_lossy(data);

    // Check if the sentence starts with '$' and contains '*'.
    if data_str.starts_with('$') && data_str.contains('*') {
        // Extract the sentence between '$' and '*'.
        let sentence: Vec<&str> = data_str.trim_start_matches('$').split('*').collect();
        let cleaned_sentence = sentence[0].trim();

        // Dispatch to specialized parsing functions based on sentence type.
        if cleaned_sentence.starts_with("GPGSV")
            || cleaned_sentence.starts_with("GLGSV")
            || cleaned_sentence.starts_with("GAGSV")
        {
            // Parse and display GSV sentence.
            parse_and_display_gsv(cleaned_sentence);
        } else if cleaned_sentence.starts_with("GNGGA") {
            // Parse and display GGA sentence.
            parse_and_display_gga(cleaned_sentence, mqtt, config);
        } else if cleaned_sentence.starts_with("GNRMC") {
            // Parse and display RMC sentence.
            parse_and_display_rmc(cleaned_sentence, mqtt, config);
        } else if cleaned_sentence.starts_with("GNVTG") {
            // Parse and display VTG sentence.
            parse_and_display_vtg(cleaned_sentence, mqtt, config);
        } else if cleaned_sentence.starts_with("GNGSA") {
            // Parse and display GSA sentence.
            parse_and_display_gsa(cleaned_sentence);
        } else if cleaned_sentence.starts_with("GNGLL") {
            // Parse and display GLL sentence.
            parse_and_display_gll(cleaned_sentence);
        } else if cleaned_sentence.starts_with("GNTXT") {
            // Parse and display GNTXT sentence.
            parse_and_display_gntxt(cleaned_sentence);
        } else {
            // Unknown sentence type, just print the raw data.
            println!("Unknown Sentence Type: {}", cleaned_sentence);
        }
    }
}

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

fn parse_and_display_gga(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 7 {
        let latitude: f64 = FromStr::from_str(parts[2]).unwrap_or(0.0);
        let longitude: f64 = FromStr::from_str(parts[4]).unwrap_or(0.0);
        let altitude: f64 = FromStr::from_str(parts[9]).unwrap_or(0.0);
        let fix_quality: usize = FromStr::from_str(parts[6]).unwrap_or(0);

        println!(
            "GGA Sentence - Latitude: {}, Longitude: {}, Altitude: {}, Fix Quality: {}",
            latitude, longitude, altitude, fix_quality
        );

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

        println!(
            "RMC Sentence - UTC Time: {:02}:{:02}:{:02}, Date: 20{}.{:02}.{:02} - Latitude: {}, Longitude: {}, Speed: {}km/h",
            hour, minute, second, year, month, day, latitude, longitude, speed
        );

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
            println!("Error pushing longitude to MQTT: {:?}", e);
        }
    } else {
        println!("Invalid RMC Sentence: {}", data);
    }
}

fn parse_utc_time(utc_time: &str) -> (u32, u32, u32) {
    // Parse UTC time in the format HHMMSS.ss
    let hour: u32 = FromStr::from_str(&utc_time[0..2]).unwrap_or(0);
    let minute: u32 = FromStr::from_str(&utc_time[2..4]).unwrap_or(0);
    let second: u32 = FromStr::from_str(&utc_time[4..6]).unwrap_or(0);

    (hour, minute, second)
}

fn parse_date(date: &str) -> (u32, u32, u32) {
    // Parse date in the format DDMMYY
    let day: u32 = FromStr::from_str(&date[0..2]).unwrap_or(0);
    let month: u32 = FromStr::from_str(&date[2..4]).unwrap_or(0);
    let year: u32 = FromStr::from_str(&date[4..6]).unwrap_or(0);

    (day, month, year)
}

fn parse_and_display_vtg(data: &str, mqtt: mqtt::Client, config: &AppConfig) {
    let parts: Vec<&str> = data.split(',').collect();
    if parts.len() >= 9 {
        let course: f64 = FromStr::from_str(parts[1]).unwrap_or(0.0);
        let speed_knots: f64 = FromStr::from_str(parts[5]).unwrap_or(0.0);
        let speed_kph: f64 = FromStr::from_str(parts[7]).unwrap_or(0.0);

        println!(
            "VTG Sentence - Course: {}, Speed (Knots): {}, Speed (KPH): {}",
            course, speed_knots, speed_kph
        );

        // Push course to MQTT
        if let Err(e) = publish_message(
            &mqtt,
            &format!("{}CRS", config.mqtt_base_topic),
            &format!("{}", course).as_str(),
            0,
        ) {
            println!("Error pushing course to MQTT: {:?}", e);
        }
    } else {
        println!("Invalid VTG Sentence: {}", data);
    }
}

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

fn parse_latitude(value: &str, direction: &str) -> f64 {
    // Ensure the input strings have sufficient length
    if value.len() >= 2 && direction.len() == 1 {
        // Extract degrees and minutes from the raw NMEA format
        let degrees: f64 = FromStr::from_str(&value[..2]).unwrap_or(0.0);
        let minutes: f64 = FromStr::from_str(&value[2..]).unwrap_or(0.0);

        // Convert to decimal degrees and consider direction
        let result = degrees + minutes / 60.0;
        if direction == "S" {
            return -result;
        } else {
            return result;
        }
    } else {
        // Handle the case where the input strings are not valid
        println!("Invalid latitude input: {}{}", value, direction);
        return 0.0;
    }
}

fn parse_longitude(value: &str, direction: &str) -> f64 {
    // Ensure the input strings have sufficient length
    if value.len() >= 3 && direction.len() == 1 {
        // Extract degrees and minutes from the raw NMEA format
        let degrees: f64 = FromStr::from_str(&value[..3]).unwrap_or(0.0);
        let minutes: f64 = FromStr::from_str(&value[3..]).unwrap_or(0.0);

        // Convert to decimal degrees and consider direction
        let result = degrees + minutes / 60.0;
        if direction == "W" {
            return -result;
        } else {
            return result;
        }
    } else {
        // Handle the case where the input strings are not valid
        println!("Invalid longitude input: {}{}", value, direction);
        return 0.0;
    }
}
