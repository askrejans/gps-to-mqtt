mod config;
mod gps_data_parser;
mod mqtt_handler;
mod serial_port_handler;

use config::load_configuration;
use config::AppConfig;
use gumdrop::Options;
use serial_port_handler::{read_from_port, setup_serial_port};

/// # GPS Data Processor
///
/// This Rust application serves as a processor for GPS data, converting it to MQTT messages.
/// It includes modules for configuration, GPS data parsing, serial port handling, and MQTT handling.
/// The main function loads configuration, sets up serial communication, and starts reading data from the port.
///
/// ## Usage
///
/// Simply run the application, and it will establish communication with the GPS device. Press 'q' to quit the application.
///
/// ## Modules
///
/// - `config`: Module for configuration settings.
/// - `gps_data_parser`: Module for parsing GPS data.
/// - `mqtt_handler`: Module for handling MQTT communication.
/// - `serial_port_handler`: Module for handling serial communication with the GPS device.
///
/// ## Functions
///
/// - `main()`: The main function that loads configuration, sets up serial communication, and starts reading data from the port.
/// - `display_welcome()`: Function to display a graphical welcome message.

/// Displays a graphical welcome message.
fn display_welcome() {
    println!("\nWelcome to GPS Data Processor!\n");
    // satellite in red
    println!(
        "\x1b[31m                                                              
                 @                                            
               @@@@@                                          
             @@@@@@@@@                                        
           @@@@@@@@@@@@                                       
           @@@@@@@@@@@@@@                                     
            @@@@@@@@@@@@@@@                                   
              @@@@@@@@@@@@@@@                                 
                @@@@@@@@@@@@@@@    @@@@                       
                  @@@@@@@@@@@@@@ @@@@@@@                      
                   @@@@@@@@@@@@@@@@@@@@@@@                    
                     @@@@@@@@@@@@@@@@@@@@@@@                  
                       @@@@@@@@@@@@    @@@@@@@                
                          @@@@@@@       @@@@@@@               
                        @@@@@@@         @@@@@@@               
                      @@@@@@@@        @@@@@@@                 
                      @@@@@@@@      @@@@@@@ @                 
@@@@   @@@@@   @@@@@    @@@@@@@@  @@@@@@@@@@@@@               
@@@@@   @@@@   @@@@@      @@@@@@@@@@@@@@@@@@@@@@@             
@@@@@   @@@@@   @@@@@       @@@@@@@@@@@@@@@@@@@@@@@           
@@@@@   @@@@@   @@@@@@        @@@@@@  @@@@@@@@@@@@@@@         
 @@@@@   @@@@@   @@@@@@@        @@     @@@@@@@@@@@@@@@@       
 @@@@@   @@@@@@   @@@@@@@@               @@@@@@@ @@@@@@@      
  @@@@@   @@@@@@@   @@@@@@@@@@@            @@@@@@@ @@@@@@@    
   @@@@@   @@@@@@@     @@@@@@@@              @@@@@@@ @@@@@@   
    @@@@@@   @@@@@@@@       @@@                @@@@@@@@@@@    
     @@@@@@    @@@@@@@@@                        @@@@@@@@@     
      @@@@@@@    @@@@@@@@@@@@@@                   @@@@@       
        @@@@@@@     @@@@@@@@@@@                     @         
         @@@@@@@@@      @@@@@@@                               
           @@@@@@@@@@@                                        
              @@@@@@@@@@@@@@@@                                
                 @@@@@@@@@@@@@@                               
                     @@@@@@@@@@                              
                     
                      \x1b[0m"
    );

    println!("==========================================");

    // Program description in green
    println!("\x1b[32mGPS to MQTT Application");
    println!("This application reads GPS data from a specified source and publishes it to an MQTT broker.");
    println!("Use the options below to interact with the application.\x1b[0m");
    println!("==========================================");
    println!("Press 'q' + Enter to quit the application.");
    println!("==========================================\n");
}

/// Define options for the program.
#[derive(Debug, Options)]
struct MyOptions {
    #[options(help = "print help message")]
    help: bool,

    #[options(help = "Sets a custom config file", meta = "FILE")]
    config: Option<String>,
}

/// Prints the help message for the GPS Data Processor application.
///
/// This function provides the user with information on how to use the application,
/// including the available commands and their descriptions. It is typically called
/// when the user requests help or uses an invalid command.
fn print_help() {
    // Help message in green
    println!("Usage: gps-to-mqtt [options]");
    println!("Options:");
    println!("  -h, --help               Print this help message");
    println!("  -c, --config FILE        Sets a custom config file path");
}

/// The main entry point of the application.
///
/// This function parses the command-line arguments, displays the welcome message,
/// loads the configuration, sets up the serial port, and starts processing data.
fn main() {
    let opts = parse_cli_args();

    if opts.help {
        print_help_and_exit();
    }

    display_welcome();

    let config = load_config_or_exit(opts.config.as_deref());

    let mut port = setup_serial_port(&config);
    read_from_port(&mut port, &config);
}

/// Parses the command-line arguments using the gumdrop crate.
///
/// This function returns the parsed options or exits the program if the arguments
/// are invalid or if the user requests help.
///
/// # Returns
///
/// * `MyOptions` - The parsed command-line options.
fn parse_cli_args() -> MyOptions {
    MyOptions::parse_args_default_or_exit()
}

/// Prints the help message and exits the program.
///
/// This function is called when the user requests help. It prints the help message
/// and then exits the program with a status code of 0.
fn print_help_and_exit() {
    print_help();
    std::process::exit(0);
}

/// Loads the configuration from the specified path or exits the program on error.
///
/// This function attempts to load the configuration from the given path. If the
/// configuration cannot be loaded, it prints an error message and exits the program
/// with a status code of 1.
///
/// # Arguments
///
/// * `config_path` - An optional path to the configuration file.
///
/// # Returns
///
/// * `AppConfig` - The loaded configuration.
fn load_config_or_exit(config_path: Option<&str>) -> AppConfig {
    match load_configuration(config_path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Error loading configuration: {}", err);
            std::process::exit(1);
        }
    }
}
