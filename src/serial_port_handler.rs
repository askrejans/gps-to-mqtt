use crate::config::AppConfig;
use crate::gps_data_parser::process_gps_data;
use crate::mqtt_handler::setup_mqtt;
use log::{error, info};
use serialport::SerialPort;
use std::io::{self, BufRead, BufReader};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// UBX-CFG-RATE command bytes for 10Hz sampling
const UBX_CFG_RATE_10HZ: [u8; 14] = [
    0xB5, 0x62, // Header
    0x06, 0x08, // Class/ID
    0x06, 0x00, // Length
    0x64, 0x00, // Measurement rate (100ms)
    0x01, 0x00, // Navigation rate
    0x01, 0x00, // Time reference
    0x7A, 0x12, // Checksum
];
const QUIT_COMMAND: &str = "q";

/// Set up and open a serial port based on the provided configuration.
///
/// This function takes an `AppConfig` reference, lists available serial ports, opens and configures
/// the specified serial port, and returns a boxed trait object representing the serial port.
///
/// # Arguments
///
/// * `config` - A reference to the `AppConfig` struct containing serial port configuration information.
///
/// # Panics
///
/// Panics if there are no available serial ports or if there is an error opening the specified port.
///
/// # Returns
///
/// Returns a boxed trait object representing the opened serial port.
pub fn setup_serial_port(config: &AppConfig) -> Box<dyn serialport::SerialPort> {
    println!("Opening port: {}", config.port_name);

    let port = serialport::new(&config.port_name, config.baud_rate as u32)
        .timeout(Duration::from_millis(1000))
        .data_bits(serialport::DataBits::Eight)
        .flow_control(serialport::FlowControl::None)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .open()
        .unwrap_or_else(|err| {
            eprintln!("Failed to open port: {}", err);
            std::process::exit(1);
        });

    if config.set_gps_to_10hz {
        println!("Setting GPS sample rate to 10Hz");
        if let Err(e) = gps_resolution_to_10hz(&mut port.try_clone().unwrap()) {
            eprintln!("Failed to set GPS sample rate: {:?}", e);
        }
    }

    port
}

/// Read data from the provided serial port and process it.
///
/// This function takes a mutable reference to a boxed trait object representing a serial port,
/// continuously reads data from the port, and processes the received data using the `process_data` function.
///
/// # Arguments
///
/// * `port` - A mutable reference to a boxed trait object representing a serial port.
pub fn read_from_port(port: &mut Box<dyn SerialPort>, config: &AppConfig) {
    let mqtt = setup_mqtt(&config);
    let (sender, receiver) = mpsc::channel();

    // Spawn quit command listener thread
    thread::spawn(move || check_quit(sender));

    // Create a buffered reader for the port
    let mut reader = BufReader::new(port.try_clone().unwrap());
    let mut line_buffer = String::with_capacity(1024);
    let mut nmea_buffer = Vec::with_capacity(1024);

    loop {
        // Check for quit command
        if let Ok(message) = receiver.try_recv() {
            if message == QUIT_COMMAND {
                println!("Received quit command. Exiting the program.");
                break;
            }
        }

        // Clear the line buffer for new data
        line_buffer.clear();

        // Read line by line
        match reader.read_line(&mut line_buffer) {
            Ok(0) => {
                // EOF reached
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Ok(_n) => {
                let line = line_buffer.trim();
                
                // Skip empty lines
                if line.is_empty() {
                    continue;
                }

                // Check for start of NMEA message
                if line.starts_with('$') {
                    // Process any complete message in buffer
                    if !nmea_buffer.is_empty() {
                        if let Err(e) = process_gps_data(&nmea_buffer, config, mqtt.clone()) {
                            eprintln!("Error processing GPS data: {:?}", e);
                        }
                        nmea_buffer.clear();
                    }
                    
                    // Start new message
                    nmea_buffer.extend_from_slice(line.as_bytes());
                    nmea_buffer.push(b'\n');
                } else if !nmea_buffer.is_empty() {
                    // Append to existing message
                    nmea_buffer.extend_from_slice(line.as_bytes());
                    nmea_buffer.push(b'\n');

                    // If we have a complete message (contains checksum)
                    if line.contains('*') {
                        if let Err(e) = process_gps_data(&nmea_buffer, config, mqtt.clone()) {
                            eprintln!("Error processing GPS data: {:?}", e);
                        }
                        nmea_buffer.clear();
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                // Timeout is normal, continue reading
                continue;
            }
            Err(e) => {
                eprintln!("Error reading from serial port: {:?}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Configures GPS device to output at 10Hz sampling rate
///
/// Sends UBX-CFG-RATE command to a ublox GPS device to set measurement
/// rate to 100ms (10Hz). Uses UBX protocol format:
/// - Header: 0xB5 0x62
/// - Class/ID: 0x06 0x08 (CFG-RATE)
/// - Payload: rate(U2), navRate(U2), timeRef(U2)
///
/// # Arguments
///
/// * `port` - Mutable reference to serial port implementing SerialPort trait
///
/// # Returns
///
/// * `io::Result<()>` - Success or IO error
///
pub fn gps_resolution_to_10hz(port: &mut Box<dyn SerialPort>) -> io::Result<()> {
    port.write_all(&UBX_CFG_RATE_10HZ).map_err(|e| {
        error!("Failed to set GPS sample rate: {}", e);
        e
    })?;

    info!("GPS sample rate configured to 10Hz");
    Ok(())
}

/// Monitors standard input for quit command ('q' + Enter)
///
/// This function runs in a separate thread and monitors stdin for user input.
/// When the quit command is detected, it sends a message through the provided channel.
///
/// # Arguments
///
/// * `sender` - Channel sender used to communicate quit command to main thread
///
/// # Example
///
/// ```
/// use std::sync::mpsc;
/// let (tx, rx) = mpsc::channel();
/// std::thread::spawn(move || check_quit(tx));
/// ```
///
/// # Notes
///
/// - Blocks until user enters input
/// - Exits when 'q' is entered or on stdin error
fn check_quit(sender: mpsc::Sender<String>) {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        match lines.next() {
            Some(Ok(line)) => {
                if line.trim() == QUIT_COMMAND {
                    if let Err(e) = sender.send(QUIT_COMMAND.to_string()) {
                        error!("Failed to send quit command: {}", e);
                        break;
                    }
                    break;
                }
            }
            Some(Err(e)) => {
                error!("Error reading from stdin: {}", e);
                break;
            }
            None => break,
        }
    }
}
