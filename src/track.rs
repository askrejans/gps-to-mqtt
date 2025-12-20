use crate::models::{LapData, TrackConfig, TrackConfigMode, TrackPoint};
use anyhow::{Context, Result};
use std::time::Instant;
use tracing::{debug, info};

/// Detector for lap timing and sector tracking
pub struct LapDetector {
    config: TrackConfig,
    lap_data: LapData,
    last_position: Option<(f64, f64)>, // (lat, lon)
    in_start_finish_zone: bool,
    sector_states: Vec<bool>, // Track which sectors we're currently in
    last_lap_timestamp: Option<Instant>,
    is_learning: bool,
    learning_start_position: Option<(f64, f64)>,
}

impl LapDetector {
    /// Create a new lap detector with the given configuration
    pub fn new(config: TrackConfig) -> Self {
        let sector_count = config.sectors.len();
        let is_learning = config.mode == TrackConfigMode::Learn;
        
        Self {
            config,
            lap_data: LapData::default(),
            last_position: None,
            in_start_finish_zone: false,
            sector_states: vec![false; sector_count],
            last_lap_timestamp: None,
            is_learning,
            learning_start_position: None,
        }
    }

    /// Update with new GPS position and return lap data if changed
    pub fn update(&mut self, latitude: f64, longitude: f64) -> Option<LapData> {
        let position = (latitude, longitude);
        
        // Handle learning mode
        if self.is_learning {
            return self.handle_learning_mode(position);
        }

        // Check if we have a configured start/finish line
        let start_finish = match &self.config.start_finish {
            Some(sf) => sf,
            None => {
                self.last_position = Some(position);
                return None;
            }
        };

        let now = Instant::now();
        let mut lap_changed = false;

        // Check if we're in the start/finish geofence
        let in_zone = is_in_geofence(latitude, longitude, start_finish);

        // Detect crossing: was out, now in
        if in_zone && !self.in_start_finish_zone {
            if let Some(last_lap_time) = self.last_lap_timestamp {
                // Calculate lap time
                let lap_duration = now.duration_since(last_lap_time);
                let lap_time_ms = lap_duration.as_millis() as u64;
                
                self.lap_data.lap_time_ms = Some(lap_time_ms);
                
                // Update best lap if this is faster
                if let Some(best) = self.lap_data.best_lap_ms {
                    if lap_time_ms < best {
                        self.lap_data.best_lap_ms = Some(lap_time_ms);
                        info!("New best lap! {:.3}s", lap_time_ms as f64 / 1000.0);
                    }
                } else {
                    self.lap_data.best_lap_ms = Some(lap_time_ms);
                }
                
                info!(
                    "Lap {} completed in {:.3}s",
                    self.lap_data.lap_number,
                    lap_time_ms as f64 / 1000.0
                );
                
                lap_changed = true;
            }
            
            // Start new lap
            self.lap_data.lap_number += 1;
            self.lap_data.current_lap_start_ms = Some(now.elapsed().as_millis() as u64);
            self.last_lap_timestamp = Some(now);
            self.sector_states.iter_mut().for_each(|s| *s = false); // Reset sector states
            
            debug!("Starting lap {}", self.lap_data.lap_number);
        }

        self.in_start_finish_zone = in_zone;

        // Check sector crossings
        if self.check_sectors(latitude, longitude) {
            lap_changed = true;
        }

        self.last_position = Some(position);

        if lap_changed {
            Some(self.lap_data.clone())
        } else {
            None
        }
    }

    /// Get current lap data
    #[allow(dead_code)]
    pub fn get_lap_data(&self) -> &LapData {
        &self.lap_data
    }

    /// Start learning mode
    #[allow(dead_code)]
    pub fn start_learning(&mut self) {
        info!("Starting learn mode - drive a full lap to define the track");
        self.is_learning = true;
        self.learning_start_position = None;
        self.config.learned_track.clear();
        self.config.mode = TrackConfigMode::Learn;
    }

    /// Stop learning mode and set the start/finish line
    #[allow(dead_code)]
    pub fn stop_learning(&mut self, radius_meters: f64) -> Result<()> {
        if !self.is_learning {
            return Ok(());
        }

        if self.config.learned_track.is_empty() {
            anyhow::bail!("No track data recorded");
        }

        // Use the first recorded position as start/finish
        let first = self.config.learned_track.first().unwrap();
        self.config.start_finish = Some(TrackPoint {
            latitude: first.latitude,
            longitude: first.longitude,
            radius_meters,
            name: "Start/Finish".to_string(),
        });

        info!(
            "Learn mode complete. Track recorded with {} points. Start/Finish at ({}, {})",
            self.config.learned_track.len(),
            first.latitude,
            first.longitude
        );

        self.is_learning = false;
        self.config.mode = TrackConfigMode::Manual; // Switch to normal mode
        self.lap_data = LapData::default(); // Reset lap data
        
        Ok(())
    }

    /// Check if currently in learning mode
    #[allow(dead_code)]
    pub fn is_learning(&self) -> bool {
        self.is_learning
    }

    /// Get the track configuration
    #[allow(dead_code)]
    pub fn get_config(&self) -> &TrackConfig {
        &self.config
    }

    /// Set track configuration
    #[allow(dead_code)]
    pub fn set_config(&mut self, config: TrackConfig) {
        self.config = config;
        self.sector_states = vec![false; self.config.sectors.len()];
        self.is_learning = self.config.mode == TrackConfigMode::Learn;
    }

    /// Handle learning mode position updates
    fn handle_learning_mode(&mut self, position: (f64, f64)) -> Option<LapData> {
        let (lat, lon) = position;

        // Record the first position
        if self.learning_start_position.is_none() {
            self.learning_start_position = Some(position);
            self.config.learned_track.push(TrackPoint {
                latitude: lat,
                longitude: lon,
                radius_meters: 10.0, // Default radius
                name: format!("Point {}", self.config.learned_track.len()),
            });
            info!("Learning started at ({}, {})", lat, lon);
            return None;
        }

        // Add position to learned track (sample every ~10 meters)
        if let Some(last) = self.config.learned_track.last() {
            let distance = calculate_distance(last.latitude, last.longitude, lat, lon);
            if distance > 10.0 {
                self.config.learned_track.push(TrackPoint {
                    latitude: lat,
                    longitude: lon,
                    radius_meters: 10.0,
                    name: format!("Point {}", self.config.learned_track.len()),
                });
                debug!("Recorded point {} at ({}, {})", self.config.learned_track.len(), lat, lon);
            }
        }

        // Check if we've completed a lap (returned to start)
        if let Some(start) = self.learning_start_position {
            let distance_to_start = calculate_distance(start.0, start.1, lat, lon);
            if distance_to_start < 20.0 && self.config.learned_track.len() > 10 {
                info!("Lap complete! Recorded {} points", self.config.learned_track.len());
                // Don't automatically stop - let user decide when to stop learning
            }
        }

        None
    }

    /// Check if any sectors were crossed
    fn check_sectors(&mut self, latitude: f64, longitude: f64) -> bool {
        let mut changed = false;

        for (i, sector) in self.config.sectors.iter().enumerate() {
            let in_sector = is_in_geofence(latitude, longitude, sector);

            // Detect crossing: was out, now in
            if in_sector && !self.sector_states[i] {
                if let Some(start_time) = self.last_lap_timestamp {
                    let sector_time = Instant::now().duration_since(start_time).as_millis() as u64;
                    
                    // Ensure we have enough space in the sector times vector
                    while self.lap_data.sector_times_ms.len() <= i {
                        self.lap_data.sector_times_ms.push(None);
                    }
                    
                    self.lap_data.sector_times_ms[i] = Some(sector_time);
                    info!("Sector {} crossed at {:.3}s", i + 1, sector_time as f64 / 1000.0);
                    changed = true;
                }
            }

            self.sector_states[i] = in_sector;
        }

        changed
    }
}

/// Check if a position is within a geofence
fn is_in_geofence(latitude: f64, longitude: f64, point: &TrackPoint) -> bool {
    let distance = calculate_distance(latitude, longitude, point.latitude, point.longitude);
    distance <= point.radius_meters
}

/// Calculate distance between two GPS coordinates using Haversine formula
/// Returns distance in meters
fn calculate_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS: f64 = 6371000.0; // Earth's radius in meters

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin() * (delta_lat / 2.0).sin()
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin() * (delta_lon / 2.0).sin();
    
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS * c
}

/// Parse GPX file and extract track points
pub fn parse_gpx_file(gpx_content: &str) -> Result<Vec<TrackPoint>> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(gpx_content);
    reader.config_mut().trim_text(true);

    let mut track_points = Vec::new();
    let mut in_trkpt = false;
    let mut current_lat = 0.0;
    let mut current_lon = 0.0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"trkpt" {
                    in_trkpt = true;
                    // Extract lat/lon from attributes
                    for attr in e.attributes() {
                        if let Ok(attr) = attr {
                            match attr.key.as_ref() {
                                b"lat" => {
                                    current_lat = std::str::from_utf8(&attr.value)
                                        .context("Invalid lat UTF-8")?
                                        .parse()
                                        .context("Invalid lat value")?;
                                }
                                b"lon" => {
                                    current_lon = std::str::from_utf8(&attr.value)
                                        .context("Invalid lon UTF-8")?
                                        .parse()
                                        .context("Invalid lon value")?;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"trkpt" && in_trkpt {
                    track_points.push(TrackPoint {
                        latitude: current_lat,
                        longitude: current_lon,
                        radius_meters: 10.0,
                        name: format!("GPX Point {}", track_points.len()),
                    });
                    in_trkpt = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("Error parsing GPX: {:?}", e),
            _ => {}
        }
        buf.clear();
    }

    if track_points.is_empty() {
        anyhow::bail!("No track points found in GPX file");
    }

    info!("Parsed {} track points from GPX", track_points.len());
    Ok(track_points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geofence_detection() {
        let point = TrackPoint {
            latitude: 40.7128,
            longitude: -74.0060,
            radius_meters: 10.0,
            name: "Test".to_string(),
        };

        // Same location - should be in geofence
        assert!(is_in_geofence(40.7128, -74.0060, &point));

        // Far away - should not be in geofence
        assert!(!is_in_geofence(41.0, -74.0, &point));
    }

    #[test]
    fn test_calculate_distance() {
        // Test distance between two known points
        let distance = calculate_distance(0.0, 0.0, 0.001, 0.001);
        assert!(distance > 100.0 && distance < 200.0); // Should be ~157 meters
    }
}
