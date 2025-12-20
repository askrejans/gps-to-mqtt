mod config;
mod logging;
mod models;
mod mqtt;
mod parser;
mod serial;
mod service;
mod telemetry;
mod track;
mod ui;

use anyhow::Result;
use clap::Parser;
use config::load_configuration;
use models::{AppMode, AppState, GpsData, MqttStatus};
use parser::GpsEvent;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

/// GPS to MQTT bridge application
#[derive(Parser, Debug)]
#[command(name = "gps-to-mqtt")]
#[command(about = "GPS to MQTT bridge", long_about = None)]
struct Args {
    /// Application mode: tui, cli, or service
    #[arg(short, long, value_enum, default_value = "tui")]
    mode: CliMode,

    /// Path to configuration file
    #[arg(short, long)]
    config: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long)]
    log_level: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum CliMode {
    Tui,
    Cli,
    Service,
}

impl From<CliMode> for AppMode {
    fn from(mode: CliMode) -> Self {
        match mode {
            CliMode::Tui => AppMode::Tui,
            CliMode::Cli => AppMode::Cli,
            CliMode::Service => AppMode::Service,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let args = Args::parse();
    let mode: AppMode = args.mode.into();

    // Load configuration
    let mut config = load_configuration(args.config.as_deref(), mode)?;

    // Override log level if specified
    if let Some(log_level) = args.log_level {
        config.log_level = log_level;
    }

    // Initialize logging
    logging::init_logging(&config)?;

    info!("Starting GPS to MQTT application in {:?} mode", config.mode);

    // Display welcome message for TUI and CLI modes
    if matches!(config.mode, AppMode::Tui | AppMode::Cli) {
        display_welcome();
    }

    // Run the application
    if let Err(e) = run_application(config).await {
        error!("Application error: {}", e);
        std::process::exit(1);
    }

    info!("Application shutdown complete");
    Ok(())
}

/// Run the main application logic
async fn run_application(config: config::AppConfig) -> Result<()> {
    // Create shared application state
    let app_state = Arc::new(RwLock::new(AppState::new()));
    let log_buffer = Arc::new(RwLock::new(Vec::<String>::new()));

    // Create channels for communication between tasks
    let (gps_event_tx, gps_event_rx) = mpsc::channel::<GpsEvent>(100);
    let (gps_data_tx, gps_data_rx) = mpsc::channel::<GpsData>(10);
    let (mqtt_status_tx, mqtt_status_rx) = mpsc::channel::<MqttStatus>(10);

    // Spawn GPS event processing task
    let state_clone = app_state.clone();
    let data_tx_clone = gps_data_tx.clone();
    tokio::spawn(async move {
        process_gps_events(gps_event_rx, state_clone, data_tx_clone).await;
    });

    // Spawn MQTT status update task
    let state_clone = app_state.clone();
    tokio::spawn(async move {
        update_mqtt_status(mqtt_status_rx, state_clone).await;
    });

    // Start serial port reading
    serial::spawn_serial_task(config.clone(), gps_event_tx).await?;
    info!("Serial port task started");

    // Start MQTT client
    mqtt::spawn_mqtt_task(config.clone(), gps_data_rx, mqtt_status_tx).await?;
    info!("MQTT task started");

    // Run mode-specific interface
    match config.mode {
        AppMode::Tui => {
            let tui_app = ui::TuiApp::new(app_state, log_buffer);
            tui_app.run(config).await?;
        }
        AppMode::Cli => {
            info!("Running in CLI mode. Press Ctrl+C to quit.");
            service::wait_for_shutdown_signal().await;
        }
        AppMode::Service => {
            info!("Running in service mode");
            service::wait_for_shutdown_signal().await;
        }
    }

    Ok(())
}

/// Process GPS events and update application state
async fn process_gps_events(
    mut event_rx: mpsc::Receiver<GpsEvent>,
    state: Arc<RwLock<AppState>>,
    data_tx: mpsc::Sender<GpsData>,
) {
    while let Some(event) = event_rx.recv().await {
        let mut state_guard = state.write().await;

        match event {
            GpsEvent::SatelliteUpdate(sat) => {
                state_guard.gps_data.update_satellite(sat);
                state_guard.serial_connected = true;
            }
            GpsEvent::NavigationUpdate(nav) => {
                // Merge navigation data
                if nav.latitude.is_some() {
                    state_guard.gps_data.navigation.latitude = nav.latitude;
                }
                if nav.longitude.is_some() {
                    state_guard.gps_data.navigation.longitude = nav.longitude;
                }
                if nav.altitude.is_some() {
                    state_guard.gps_data.navigation.altitude = nav.altitude;
                }
                if nav.speed_knots.is_some() {
                    state_guard.gps_data.navigation.speed_knots = nav.speed_knots;
                }
                if nav.speed_kph.is_some() {
                    state_guard.gps_data.navigation.speed_kph = nav.speed_kph;
                }
                if nav.course.is_some() {
                    state_guard.gps_data.navigation.course = nav.course;
                }
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::FixUpdate(fix) => {
                // Merge fix data
                if fix.fix_type.is_some() {
                    state_guard.gps_data.fix.fix_type = fix.fix_type;
                }
                if fix.fix_quality.is_some() {
                    state_guard.gps_data.fix.fix_quality = fix.fix_quality;
                }
                if fix.satellites_used.is_some() {
                    state_guard.gps_data.fix.satellites_used = fix.satellites_used;
                }
                if fix.hdop.is_some() {
                    state_guard.gps_data.fix.hdop = fix.hdop;
                }
                if fix.vdop.is_some() {
                    state_guard.gps_data.fix.vdop = fix.vdop;
                }
                if fix.pdop.is_some() {
                    state_guard.gps_data.fix.pdop = fix.pdop;
                }
                if fix.time.is_some() {
                    state_guard.gps_data.fix.time = fix.time;
                }
                if fix.date.is_some() {
                    state_guard.gps_data.fix.date = fix.date;
                }
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::Message(msg) => {
                state_guard.gps_data.add_message(msg);
                state_guard.serial_connected = true;
            }
            GpsEvent::AccuracyUpdate { std_lat, std_lon, std_alt: _ } => {
                // Calculate overall position accuracy and store in navigation data
                let position_accuracy = (std_lat * std_lat + std_lon * std_lon).sqrt();
                state_guard.gps_data.navigation.position_accuracy = Some(position_accuracy);
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::RateOfTurn(_rate) => {
                // Rate of turn data parsed but not currently used for calculations
                // Available for future enhancements
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::TrueHeading(heading) => {
                state_guard.gps_data.navigation.true_heading = Some(heading);
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
        }

        // Update satellite count
        state_guard.gps_data.satellites_in_view = Some(state_guard.gps_data.satellites.len() as u32);

        // Send updated GPS data to MQTT publisher
        if let Err(e) = data_tx.send(state_guard.gps_data.clone()).await {
            warn!("Failed to send GPS data to MQTT publisher: {}", e);
        }
    }

    info!("GPS event processing task ended");
}

/// Update MQTT connection status
async fn update_mqtt_status(
    mut status_rx: mpsc::Receiver<MqttStatus>,
    state: Arc<RwLock<AppState>>,
) {
    while let Some(status) = status_rx.recv().await {
        let mut state_guard = state.write().await;
        state_guard.mqtt_status = status;
        info!("MQTT status changed to: {:?}", status);
    }
}

/// Display welcome message
fn display_welcome() {
    println!("\n╔═══════════════════════════════════════════╗");
    println!("║     GPS to MQTT Bridge                    ║");
    println!("╚═══════════════════════════════════════════╝");
    println!();
    println!("📡 Connecting to GPS receiver...");
    println!("🌐 Establishing MQTT connection...");
    println!();
}
