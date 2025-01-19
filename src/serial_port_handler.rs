use crate::config::AppConfig;
use crate::gps_data_parser::process_gps_data;
use crate::mqtt_handler::setup_mqtt;
use log::{error, info};
use serialport::SerialPort;
use std::io::{self, BufRead};
use std::sync::mpsc;
use std::thread;

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

    let mut port = serialport::new(&config.port_name, config.baud_rate as u32)
        .timeout(std::time::Duration::from_millis(1000))
        .open()
        .unwrap_or_else(|err| {
            eprintln!("Failed to open port: {}", err);
            std::process::exit(1);
        });

    if config.set_gps_to_10hz {
        println!("Setting GPS sample rate to 10Hz");
        if let Err(e) = gps_resolution_to_10hz(&mut port) {
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
    let mut serial_buf = vec![0; 1024];
    let mqtt = setup_mqtt(&config);

    let (sender, receiver) = mpsc::channel();

    thread::spawn({
        let sender = sender.clone();
        move || check_quit(sender)
    });

    loop {
        if let Ok(message) = receiver.try_recv() {
            if message == "q" {
                println!("Received quit command. Exiting the program.");
                break;
            }
        }

        match port.read(serial_buf.as_mut_slice()) {
            Ok(t) if t > 0 => {
                let data = &serial_buf[..t];
                if let Err(e) = process_gps_data(data, config, mqtt.clone()) {
                    eprintln!("Error processing GPS data: {:?}", e);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => (),
            Err(e) => eprintln!("Serial port read error: {:?}", e),
            _ => (),
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
