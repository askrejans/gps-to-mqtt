use crate::config::AppConfig;
use crate::models::{GpsData, MqttStatus, TrackConfig, TrackConfigMode, TrackPoint};
use crate::telemetry::TelemetryCalculator;
use crate::track::{LapDetector, parse_gpx_file};
use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

/// Message to be published to MQTT
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

/// MQTT client wrapper with reconnection logic
pub struct MqttClient {
    client: AsyncClient,
    eventloop: EventLoop,
    config: AppConfig,
    last_published: Arc<RwLock<HashMap<String, String>>>,
}

impl MqttClient {
    /// Create a new MQTT client
    pub fn new(config: AppConfig) -> Result<Self> {
        let mut mqttoptions = MqttOptions::new(
            &config.mqtt_client_id,
            &config.mqtt_host,
            config.mqtt_port,
        );

        mqttoptions.set_keep_alive(Duration::from_secs(30));
        mqttoptions.set_clean_session(true);

        let (client, eventloop) = AsyncClient::new(mqttoptions, 10);

        Ok(Self {
            client,
            eventloop,
            config,
            last_published: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Publish a message with change detection (only publishes if value changed)
    #[allow(dead_code)]
    pub async fn publish_with_change_detection(&self, topic: &str, payload: String) -> Result<()> {
        let full_topic = format!("{}{}", self.config.mqtt_base_topic, topic);
        
        // Check if value changed
        let mut last_values = self.last_published.write().await;
        if let Some(last_value) = last_values.get(&full_topic) {
            if last_value == &payload {
                debug!("Skipping unchanged value for topic: {}", full_topic);
                return Ok(());
            }
        }

        // Publish the message
        self.client
            .publish(&full_topic, QoS::AtMostOnce, true, payload.clone())
            .await
            .context("Failed to publish MQTT message")?;

        // Update last published value
        last_values.insert(full_topic.clone(), payload);
        debug!("Published to {}", full_topic);

        Ok(())
    }

    /// Publish a message without change detection
    #[allow(dead_code)]
    pub async fn publish(&self, topic: &str, payload: String, retain: bool) -> Result<()> {
        let full_topic = format!("{}{}", self.config.mqtt_base_topic, topic);
        
        self.client
            .publish(&full_topic, QoS::AtMostOnce, retain, payload)
            .await
            .context("Failed to publish MQTT message")?;

        debug!("Published to {}", full_topic);
        Ok(())
    }

    /// Run the MQTT event loop with reconnection
    pub async fn run_event_loop(
        mut self,
        status_tx: mpsc::Sender<MqttStatus>,
    ) {
        let mut reconnect_delay = RECONNECT_BASE_DELAY;
        let mut attempt = 0;

        loop {
            // Send connecting status
            let _ = status_tx.send(MqttStatus::Connecting).await;
            info!("Connecting to MQTT broker at {}:{}", self.config.mqtt_host, self.config.mqtt_port);

            // Process events
            let result = self.process_events(&status_tx).await;

            match result {
                Ok(_) => {
                    info!("MQTT connection closed gracefully");
                    break;
                }
                Err(e) => {
                    error!("MQTT connection error: {}", e);
                    let _ = status_tx.send(MqttStatus::Error).await;

                    // Check max attempts
                    if self.config.mqtt_reconnect_max_attempts > 0 {
                        attempt += 1;
                        if attempt >= self.config.mqtt_reconnect_max_attempts {
                            error!("Max reconnection attempts reached, giving up");
                            break;
                        }
                    }

                    // Exponential backoff
                    warn!("Reconnecting in {:?}...", reconnect_delay);
                    tokio::time::sleep(reconnect_delay).await;
                    reconnect_delay = std::cmp::min(reconnect_delay * 2, RECONNECT_MAX_DELAY);
                }
            }
        }

        let _ = status_tx.send(MqttStatus::Disconnected).await;
    }

    /// Process MQTT events
    async fn process_events(&mut self, status_tx: &mpsc::Sender<MqttStatus>) -> Result<()> {
        loop {
            match self.eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("Connected to MQTT broker");
                    let _ = status_tx.send(MqttStatus::Connected).await;
                }
                Ok(Event::Incoming(packet)) => {
                    debug!("Received MQTT packet: {:?}", packet);
                }
                Ok(Event::Outgoing(_)) => {
                    // Outgoing packets, nothing to do
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("MQTT event loop error: {}", e));
                }
            }
        }
    }
}

/// Spawn the MQTT publishing task
pub async fn spawn_mqtt_task(
    config: AppConfig,
    mut gps_rx: mpsc::Receiver<GpsData>,
    status_tx: mpsc::Sender<MqttStatus>,
) -> Result<()> {
    let client = MqttClient::new(config.clone())?;
    let publish_client = client.client.clone();
    let base_topic = config.mqtt_base_topic.clone();
    let last_published = client.last_published.clone();

    // Spawn event loop task
    tokio::spawn(async move {
        client.run_event_loop(status_tx).await;
    });

    // Initialize telemetry calculator if enabled
    let mut telemetry_calc = if config.telemetry_enabled {
        Some(TelemetryCalculator::new(config.telemetry_smoothing_window))
    } else {
        None
    };

    // Initialize lap detector based on track mode
    let mut lap_detector = initialize_lap_detector(&config)?;

    // Spawn publishing task
    tokio::spawn(async move {
        while let Some(gps_data) = gps_rx.recv().await {
            // Publish standard GPS data
            if let Err(e) = publish_gps_data(&publish_client, &base_topic, &gps_data, &last_published).await {
                warn!("Failed to publish GPS data: {}", e);
            }

            // Calculate and publish telemetry metrics if enabled
            if let Some(ref mut calc) = telemetry_calc {
                let metrics = calc.update(&gps_data);
                if let Err(e) = publish_telemetry_data(&publish_client, &base_topic, &metrics, &last_published).await {
                    warn!("Failed to publish telemetry data: {}", e);
                }
            }

            // Update lap detector and publish lap data if changed
            if let Some(ref mut detector) = lap_detector {
                if let (Some(lat), Some(lon)) = (gps_data.navigation.latitude, gps_data.navigation.longitude) {
                    if let Some(lap_data) = detector.update(lat, lon) {
                        if let Err(e) = publish_lap_data(&publish_client, &base_topic, &lap_data, &last_published).await {
                            warn!("Failed to publish lap data: {}", e);
                        }
                    }
                }
            }
        }
        info!("MQTT publishing task ended");
    });

    Ok(())
}

/// Initialize lap detector based on configuration
fn initialize_lap_detector(config: &AppConfig) -> Result<Option<LapDetector>> {
    let track_config = match config.track_mode.as_str() {
        "disabled" => return Ok(None),
        "manual" => {
            // Manual configuration
            if let (Some(lat), Some(lon)) = (config.track_start_lat, config.track_start_lon) {
                let start_finish = TrackPoint {
                    latitude: lat,
                    longitude: lon,
                    radius_meters: config.track_geofence_radius,
                    name: "Start/Finish".to_string(),
                };
                TrackConfig {
                    mode: TrackConfigMode::Manual,
                    start_finish: Some(start_finish),
                    sectors: Vec::new(),
                    learned_track: Vec::new(),
                }
            } else {
                warn!("Manual track mode selected but coordinates not configured");
                return Ok(None);
            }
        }
        "learn" => {
            // Learning mode
            TrackConfig {
                mode: TrackConfigMode::Learn,
                start_finish: None,
                sectors: Vec::new(),
                learned_track: Vec::new(),
            }
        }
        "gpx" => {
            // Load from GPX file
            if let Some(ref gpx_path) = config.track_gpx_file {
                match std::fs::read_to_string(gpx_path) {
                    Ok(gpx_content) => {
                        match parse_gpx_file(&gpx_content) {
                            Ok(points) => {
                                if let Some(first_point) = points.first() {
                                    TrackConfig {
                                        mode: TrackConfigMode::Gpx,
                                        start_finish: Some(first_point.clone()),
                                        sectors: Vec::new(),
                                        learned_track: points,
                                    }
                                } else {
                                    warn!("GPX file contains no track points");
                                    return Ok(None);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse GPX file: {}", e);
                                return Ok(None);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read GPX file: {}", e);
                        return Ok(None);
                    }
                }
            } else {
                warn!("GPX track mode selected but file path not configured");
                return Ok(None);
            }
        }
        _ => {
            warn!("Unknown track mode: {}", config.track_mode);
            return Ok(None);
        }
    };

    Ok(Some(LapDetector::new(track_config)))
}

/// Publish GPS data to MQTT
async fn publish_gps_data(
    client: &AsyncClient,
    base_topic: &str,
    gps_data: &GpsData,
    last_published: &Arc<RwLock<HashMap<String, String>>>,
) -> Result<()> {
    // Publish navigation data
    if let Some(lat) = gps_data.navigation.latitude {
        publish_if_changed(client, base_topic, "/LAT", lat.to_string(), last_published).await?;
    }
    if let Some(lon) = gps_data.navigation.longitude {
        publish_if_changed(client, base_topic, "/LNG", lon.to_string(), last_published).await?;
    }
    if let Some(alt) = gps_data.navigation.altitude {
        publish_if_changed(client, base_topic, "/ALT", alt.to_string(), last_published).await?;
    }
    if let Some(speed) = gps_data.navigation.speed_kph {
        publish_if_changed(client, base_topic, "/SPD_KPH", speed.to_string(), last_published).await?;
        // Also publish as SPD for backwards compatibility
        publish_if_changed(client, base_topic, "/SPD", speed.to_string(), last_published).await?;
    }
    if let Some(speed_knots) = gps_data.navigation.speed_knots {
        publish_if_changed(client, base_topic, "/SPD_KTS", speed_knots.to_string(), last_published).await?;
    }
    if let Some(course) = gps_data.navigation.course {
        publish_if_changed(client, base_topic, "/CRS", course.to_string(), last_published).await?;
    }

    // Publish fix data
    if let Some(sats) = gps_data.fix.satellites_used {
        publish_if_changed(client, base_topic, "/SATS", sats.to_string(), last_published).await?;
    }
    if let Some(hdop) = gps_data.fix.hdop {
        publish_if_changed(client, base_topic, "/HDOP", hdop.to_string(), last_published).await?;
    }
    if let Some(vdop) = gps_data.fix.vdop {
        publish_if_changed(client, base_topic, "/VDOP", format!("{:.2}", vdop), last_published).await?;
    }
    if let Some(pdop) = gps_data.fix.pdop {
        publish_if_changed(client, base_topic, "/PDOP", format!("{:.2}", pdop), last_published).await?;
    }
    if let Some(time) = gps_data.fix.time {
        publish_if_changed(client, base_topic, "/TME", time.to_string(), last_published).await?;
    }
    if let Some(date) = gps_data.fix.date {
        publish_if_changed(client, base_topic, "/DTE", date.to_string(), last_published).await?;
    }
    if let Some(ref quality) = gps_data.fix.fix_quality {
        // Convert FixQuality enum to number matching old behavior
        let quality_num = match quality {
            crate::models::FixQuality::Invalid => 0,
            crate::models::FixQuality::GpsFix => 1,
            crate::models::FixQuality::DgpsFix => 2,
            crate::models::FixQuality::PpsFix => 3,
            crate::models::FixQuality::Rtk => 4,
            crate::models::FixQuality::FloatRtk => 5,
            crate::models::FixQuality::Estimated => 6,
            crate::models::FixQuality::Manual => 7,
            crate::models::FixQuality::Simulation => 8,
        };
        publish_if_changed(client, base_topic, "/QTY", quality_num.to_string(), last_published).await?;
    }

    // Publish total satellite count
    if let Some(count) = gps_data.satellites_in_view {
        publish_if_changed(client, base_topic, "/SAT/GLOBAL/NUM", count.to_string(), last_published).await?;
    }

    // Publish position accuracy if available
    if let Some(accuracy) = gps_data.navigation.position_accuracy {
        publish_if_changed(client, base_topic, "/POSITION_ACCURACY", format!("{:.2}", accuracy), last_published).await?;
    }

    // Publish true heading if available
    if let Some(heading) = gps_data.navigation.true_heading {
        publish_if_changed(client, base_topic, "/TRUE_HEADING", format!("{:.1}", heading), last_published).await?;
    }

    // Publish heading rate if available
    if let Some(heading_rate) = gps_data.navigation.heading_rate {
        publish_if_changed(client, base_topic, "/HEADING_RATE_GPS", format!("{:.2}", heading_rate), last_published).await?;
    }

    // Publish individual satellite data
    for (prn, sat) in &gps_data.satellites {
        let sat_topic = format!("/SAT/VEHICLES/{}", prn);
        
        // Create satellite info string matching old format: "PRN: X, Type: Y, Elevation: Z, Azimuth: A, SNR: S, In View: true/false"
        let in_view = sat.snr.unwrap_or(0) > 0;
        let sat_info = format!(
            "PRN: {}, Type: {:?}, Elevation: {}, Azimuth: {}, SNR: {}, In View: {}",
            prn,
            sat.system,
            sat.elevation.map(|e| e.to_string()).unwrap_or_else(|| "N/A".to_string()),
            sat.azimuth.map(|a| a.to_string()).unwrap_or_else(|| "N/A".to_string()),
            sat.snr.map(|s| s.to_string()).unwrap_or_else(|| "N/A".to_string()),
            in_view
        );
        
        publish_if_changed(client, base_topic, &sat_topic, sat_info, last_published).await?;
    }

    Ok(())
}

/// Publish telemetry metrics to MQTT
async fn publish_telemetry_data(
    client: &AsyncClient,
    base_topic: &str,
    metrics: &crate::models::TelemetryMetrics,
    last_published: &Arc<RwLock<HashMap<String, String>>>,
) -> Result<()> {
    // Publish longitudinal acceleration
    if let Some(accel) = metrics.longitudinal_accel {
        publish_if_changed(client, base_topic, "/ACCELERATION", format!("{:.3}", accel), last_published).await?;
    }

    // Publish lateral acceleration (g-forces)
    if let Some(lateral) = metrics.lateral_accel {
        let lateral_g = lateral / 9.81; // Convert m/s² to g
        publish_if_changed(client, base_topic, "/LATERAL_G", format!("{:.3}", lateral_g), last_published).await?;
    }

    // Publish combined g-force
    if let Some(combined) = metrics.combined_g {
        publish_if_changed(client, base_topic, "/COMBINED_G", format!("{:.3}", combined), last_published).await?;
    }

    // Publish heading rate
    if let Some(heading_rate) = metrics.heading_rate {
        publish_if_changed(client, base_topic, "/HEADING_RATE", format!("{:.2}", heading_rate), last_published).await?;
    }

    // Publish distance traveled
    publish_if_changed(client, base_topic, "/DISTANCE", format!("{:.1}", metrics.distance_traveled), last_published).await?;

    // Publish max speed
    if let Some(max_speed) = metrics.max_speed_kph {
        publish_if_changed(client, base_topic, "/MAX_SPEED", format!("{:.1}", max_speed), last_published).await?;
    }

    // Publish braking status
    publish_if_changed(client, base_topic, "/BRAKING", if metrics.is_braking { "1" } else { "0" }.to_string(), last_published).await?;

    Ok(())
}

/// Publish lap data to MQTT
async fn publish_lap_data(
    client: &AsyncClient,
    base_topic: &str,
    lap_data: &crate::models::LapData,
    last_published: &Arc<RwLock<HashMap<String, String>>>,
) -> Result<()> {
    // Publish lap number
    publish_if_changed(client, base_topic, "/LAP_NUMBER", lap_data.lap_number.to_string(), last_published).await?;

    // Publish current lap time
    if let Some(lap_time) = lap_data.lap_time_ms {
        let lap_time_sec = lap_time as f64 / 1000.0;
        publish_if_changed(client, base_topic, "/LAP_TIME", format!("{:.3}", lap_time_sec), last_published).await?;
    }

    // Publish best lap time
    if let Some(best_lap) = lap_data.best_lap_ms {
        let best_lap_sec = best_lap as f64 / 1000.0;
        publish_if_changed(client, base_topic, "/BEST_LAP", format!("{:.3}", best_lap_sec), last_published).await?;
    }

    // Publish sector times
    for (i, sector_time) in lap_data.sector_times_ms.iter().enumerate() {
        if let Some(time) = sector_time {
            let sector_sec = *time as f64 / 1000.0;
            publish_if_changed(client, base_topic, &format!("/SECTOR_{}", i + 1), format!("{:.3}", sector_sec), last_published).await?;
        }
    }

    Ok(())
}

/// Publish with change detection
async fn publish_if_changed(
    client: &AsyncClient,
    base_topic: &str,
    subtopic: &str,
    payload: String,
    last_published: &Arc<RwLock<HashMap<String, String>>>,
) -> Result<()> {
    let full_topic = format!("{}{}", base_topic, subtopic);
    
    // Check if value changed
    let mut last_values = last_published.write().await;
    if let Some(last_value) = last_values.get(&full_topic) {
        if last_value == &payload {
            return Ok(());
        }
    }

    // Publish the message
    client
        .publish(&full_topic, QoS::AtMostOnce, true, payload.clone())
        .await
        .context("Failed to publish MQTT message")?;

    // Update last published value
    last_values.insert(full_topic, payload);

    Ok(())
}
