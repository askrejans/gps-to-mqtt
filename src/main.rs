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
use config::load_configuration;
use gumdrop::Options;
use logging::TuiWriter;
use models::{AppState, GpsData, MqttStatus};
use parser::GpsEvent;
use std::sync::Arc as StdArc;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[derive(Debug, Options)]
struct Opts {
    #[options(help = "print help message")]
    help: bool,

    #[options(short = "c", help = "path to configuration file")]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::parse_args_default_or_exit();

    let config = load_configuration(opts.config.as_deref())?;

    let is_tty = atty::is(atty::Stream::Stdout);

    let cancel = CancellationToken::new();

    // Initialise logging — TUI mode captures logs into a ring buffer shown in the UI;
    // service / daemon mode writes structured logs to stdout.
    let log_buffer: Option<Arc<Mutex<VecDeque<String>>>> = if is_tty {
        let buf = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(
            config.max_log_buffer_size,
        )));
        logging::init_logging_tui(&config, TuiWriter::new(Arc::clone(&buf)));
        Some(buf)
    } else {
        display_welcome();
        logging::init_logging_service(&config);
        None
    };

    info!("Starting GPS-to-MQTT v{}", env!("CARGO_PKG_VERSION"));

    // Shared application state
    let app_state = Arc::new(RwLock::new(AppState {
        mqtt_enabled: config.mqtt_enabled,
        connection_address: config.connection_display(),
        mqtt_address: if config.mqtt_enabled {
            config.mqtt_address()
        } else {
            String::new()
        },
        ..AppState::default()
    }));

    // Channels
    let (gps_event_tx, gps_event_rx) = mpsc::channel::<GpsEvent>(100);
    let (gps_data_tx, gps_data_rx) = mpsc::channel::<GpsData>(10);
    let (mqtt_status_tx, mqtt_status_rx) = mpsc::channel::<MqttStatus>(10);

    // GPS event → state + forward to MQTT
    {
        let state = Arc::clone(&app_state);
        tokio::spawn(async move {
            process_gps_events(gps_event_rx, state, gps_data_tx).await;
        });
    }

    // MQTT status → state
    {
        let state = Arc::clone(&app_state);
        tokio::spawn(async move {
            update_mqtt_status(mqtt_status_rx, state).await;
        });
    }

    // Start serial port
    serial::spawn_serial_task(config.clone(), gps_event_tx, cancel.clone()).await?;
    info!("Serial port task started");

    // Start MQTT client (optional)
    if config.mqtt_enabled {
        let msg_counter = StdArc::clone(&app_state.read().await.messages_published);
        mqtt::spawn_mqtt_task(config.clone(), gps_data_rx, mqtt_status_tx, msg_counter).await?;
        info!("MQTT task started");
    } else {
        info!("MQTT disabled — running in display-only mode");
        // Drop unused sender so the channel cleanly closes
        drop(gps_data_rx);
        drop(mqtt_status_tx);
    }

    // Spawn signal listener — cancels the token on SIGTERM / Ctrl+C
    {
        let c = cancel.clone();
        tokio::spawn(async move {
            service::wait_for_shutdown_signal().await;
            c.cancel();
        });
    }

    // Wait: TUI blocks until quit; service mode waits for the cancel token
    if is_tty {
        let buf = log_buffer.expect("log_buffer is Some in TUI mode");
        if let Err(e) = ui::run_tui(Arc::clone(&app_state), buf, cancel.clone()).await {
            error!("TUI error: {}", e);
        }
        cancel.cancel();
    } else {
        cancel.cancelled().await;
    }

    info!("Application shutdown complete");
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
            GpsEvent::RawNmea(sentence) => {
                state_guard.gps_data.add_raw_nmea(sentence);
                state_guard.serial_connected = true;
            }
            GpsEvent::AccuracyUpdate {
                std_lat,
                std_lon,
                std_alt: _,
            } => {
                let position_accuracy = (std_lat * std_lat + std_lon * std_lon).sqrt();
                state_guard.gps_data.navigation.position_accuracy = Some(position_accuracy);
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::RateOfTurn(rate) => {
                state_guard.gps_data.navigation.heading_rate = Some(rate / 60.0); // deg/min → deg/s
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
            GpsEvent::TrueHeading(heading) => {
                state_guard.gps_data.navigation.true_heading = Some(heading);
                state_guard.gps_data.last_update = Some(std::time::Instant::now());
                state_guard.serial_connected = true;
            }
        }

        state_guard.gps_data.satellites_in_view =
            Some(state_guard.gps_data.satellites.len() as u32);

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
        info!("MQTT status: {:?}", status);
    }
}

fn display_welcome() {
    println!("\n╔═══════════════════════════════════════════╗");
    println!("║        GPS to MQTT Bridge                 ║");
    println!("╚═══════════════════════════════════════════╝\n");
}
