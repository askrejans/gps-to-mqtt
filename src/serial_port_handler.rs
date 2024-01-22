use crate::config::AppConfig;
use crate::gps_data_parser::process_gps_data;
use crate::mqtt_handler::setup_mqtt;
use serialport::SerialPort;
use std::io::BufRead;
use std::io;
use std::sync::mpsc;
use std::thread;

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
pub fn setup_serial_port(config: &AppConfig) -> Box<dyn SerialPort> {

    // Opening and configuring the specified serial port.
    println!("Opening port: {}", config.port_name);
    let mut port = serialport::new(&config.port_name, config.baud_rate as u32)
        .timeout(std::time::Duration::from_millis(1000))
        .open()
        .expect("Failed to open port");

    if config.set_gps_to_10hz {
        println!("Setting GPS sample rate to 10Hz");
        gps_resolution_to_10hz(&mut port);
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
    // Buffer to store received serial data.
    let mut serial_buf: Vec<u8> = vec![0; 1024];
    let mqtt = setup_mqtt(&config);

    // Create a channel for communication between threads
    let (sender, receiver) = mpsc::channel();
    let sender_clone = sender.clone();

    // Spawn a separate thread for user input
    thread::spawn(move || {
        check_quit(sender_clone);
    });

    // Continuously read data from the serial port.
    loop {
        // Check if the user wants to quit.
        if let Ok(message) = receiver.try_recv() {
            if message == "q" {
                println!("Received quit command. Exiting the program.");
                return;
            }
        }

        match port.read(serial_buf.as_mut_slice()) {
            Ok(t) => {
                if t > 0 {
                    // Process and print the received data.
                    let data = &serial_buf[0..t];
                    process_gps_data(data, config, mqtt.clone());
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => (),
            Err(e) => eprintln!("{:?}", e),
        }
    }
}

/// Increases GPS resolution to 10Hz (ublox GPS device)
///
/// UBX -> CFG -> RATE command to 10Hz
///
/// # Arguments
///
/// * `port` - A mutable reference to a boxed trait object representing a serial port.
pub fn gps_resolution_to_10hz(port: &mut Box<dyn SerialPort>) {
    // Bytes to send to the device.
    let bytes_to_send: Vec<u8> = vec![
        0xB5, 0x62, 0x06, 0x08, 0x06, 0x00, 0x64, 0x00, 0x01, 0x00, 0x01, 0x00, 0x7A, 0x12,
    ];

    // Send the bytes to the device.
    match port.write_all(&bytes_to_send) {
        Ok(_) => {
            println!("Sample rate set successfully!");
        }
        Err(e) => {
            eprintln!("Error sending bytes to set sample rate: {:?}", e);
        }
    }
}

/// Check if the user wants to quit by entering 'q' + Enter.
fn check_quit(sender: mpsc::Sender<String>) {
    // Create a buffer to read user input
    let mut input_buffer = String::new();

    // Read input from the user asynchronously
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    // Check if the input is 'q'
    if let Some(Ok(line)) = lines.next() {
        if line.trim() == "q" {
            // Send quit command to the main thread
            sender.send("q".to_string()).unwrap();
            return;
        }
    }

    // If the input is not 'q', continue checking
    check_quit(sender);
}