use crate::models::{GnssSystem, GpsData};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use ratatui::widgets::canvas::Canvas;
use std::f64::consts::PI;

/// Create position information widget
pub fn create_position_widget(gps_data: &GpsData) -> Paragraph<'static> {
    let lat = gps_data
        .navigation
        .latitude
        .map(|v| format!("{:.6}°", v))
        .unwrap_or_else(|| "N/A".to_string());

    let lon = gps_data
        .navigation
        .longitude
        .map(|v| format!("{:.6}°", v))
        .unwrap_or_else(|| "N/A".to_string());

    let alt = gps_data
        .navigation
        .altitude
        .map(|v| format!("{:.1} m", v))
        .unwrap_or_else(|| "N/A".to_string());

    let speed = gps_data
        .navigation
        .speed_kph
        .map(|v| format!("{:.1} km/h", v))
        .unwrap_or_else(|| "N/A".to_string());

    let course = gps_data
        .navigation
        .course
        .map(|v| format!("{:.1}°", v))
        .unwrap_or_else(|| "N/A".to_string());

    let text = vec![
        Line::from(vec![
            Span::styled("Latitude:  ", Style::default().fg(Color::Cyan)),
            Span::raw(lat),
        ]),
        Line::from(vec![
            Span::styled("Longitude: ", Style::default().fg(Color::Cyan)),
            Span::raw(lon),
        ]),
        Line::from(vec![
            Span::styled("Altitude:  ", Style::default().fg(Color::Cyan)),
            Span::raw(alt),
        ]),
        Line::from(vec![
            Span::styled("Speed:     ", Style::default().fg(Color::Cyan)),
            Span::raw(speed),
        ]),
        Line::from(vec![
            Span::styled("Course:    ", Style::default().fg(Color::Cyan)),
            Span::raw(course),
        ]),
    ];

    Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Position & Navigation"),
    )
}

/// Create fix information widget
pub fn create_fix_widget(gps_data: &GpsData) -> Paragraph<'static> {
    let fix_type = gps_data
        .fix
        .fix_type
        .as_ref()
        .map(|f| format!("{:?}", f))
        .unwrap_or_else(|| "Unknown".to_string());

    let fix_quality = gps_data
        .fix
        .fix_quality
        .as_ref()
        .map(|q| format!("{:?}", q))
        .unwrap_or_else(|| "Unknown".to_string());

    let sats_used = gps_data
        .fix
        .satellites_used
        .map(|s| s.to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let sats_view = gps_data
        .satellites_in_view
        .map(|s| s.to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let hdop = gps_data
        .fix
        .hdop
        .map(|h| format!("{:.2}", h))
        .unwrap_or_else(|| "N/A".to_string());

    let time = gps_data
        .fix
        .time
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let text = vec![
        Line::from(vec![
            Span::styled("Fix Type:    ", Style::default().fg(Color::Cyan)),
            Span::raw(fix_type),
        ]),
        Line::from(vec![
            Span::styled("Fix Quality: ", Style::default().fg(Color::Cyan)),
            Span::raw(fix_quality),
        ]),
        Line::from(vec![
            Span::styled("Sats Used:   ", Style::default().fg(Color::Cyan)),
            Span::raw(sats_used),
        ]),
        Line::from(vec![
            Span::styled("Sats in View:", Style::default().fg(Color::Cyan)),
            Span::raw(sats_view),
        ]),
        Line::from(vec![
            Span::styled("HDOP:        ", Style::default().fg(Color::Cyan)),
            Span::raw(hdop),
        ]),
        Line::from(vec![
            Span::styled("Time:        ", Style::default().fg(Color::Cyan)),
            Span::raw(time),
        ]),
    ];

    Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Fix Information"))
}

/// Create messages widget
#[allow(dead_code)]
pub fn create_messages_widget(gps_data: &GpsData) -> Paragraph {
    let messages: Vec<Line> = gps_data
        .messages
        .iter()
        .rev()
        .take(20)
        .map(|msg| Line::from(msg.as_str()))
        .collect();

    Paragraph::new(messages).block(Block::default().borders(Borders::ALL).title("Messages"))
}

/// Create satellite list widget
pub fn create_satellite_list_widget(gps_data: &GpsData) -> List<'static> {
    let satellites_by_system = gps_data.satellites_by_system();

    let mut items = Vec::new();

    // Add header
    items.push(ListItem::new(Line::from(vec![
        Span::styled("PRN ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("El  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("Az  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("SNR ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("Sys", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ])));

    // Group by system
    for (system, sats) in satellites_by_system.iter() {
        let system_color = match system {
            GnssSystem::Gps => Color::Green,
            GnssSystem::Glonass => Color::Blue,
            GnssSystem::Galileo => Color::Cyan,
            GnssSystem::Beidou => Color::Magenta,
            GnssSystem::Unknown => Color::Gray,
        };

        for sat in sats.iter() {
            let prn = format!("{:<4}", sat.prn);
            let el = sat
                .elevation
                .map(|e| format!("{:<4}", e))
                .unwrap_or_else(|| "N/A ".to_string());
            let az = sat
                .azimuth
                .map(|a| format!("{:<4}", a))
                .unwrap_or_else(|| "N/A ".to_string());
            let snr = sat
                .snr
                .map(|s| format!("{:<4}", s))
                .unwrap_or_else(|| "N/A ".to_string());

            let snr_color = sat.snr.map(|s| {
                if s >= 40 {
                    Color::Green
                } else if s >= 30 {
                    Color::Yellow
                } else {
                    Color::Red
                }
            }).unwrap_or(Color::Gray);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(prn, Style::default().fg(system_color)),
                Span::raw(el),
                Span::raw(az),
                Span::styled(snr, Style::default().fg(snr_color)),
                Span::styled(format!("{:?}", system), Style::default().fg(system_color)),
            ])));
        }
    }

    if items.len() == 1 {
        items.push(ListItem::new("No satellites in view"));
    }

    List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Satellite Details"),
    )
}

/// Create satellite sky chart widget
pub fn create_satellite_sky_chart(gps_data: &GpsData) -> Canvas<'static, impl Fn(&mut ratatui::widgets::canvas::Context) + 'static> {
    let satellites = gps_data.satellites.clone();

    Canvas::default()
        .block(Block::default().borders(Borders::ALL).title("Sky View"))
        .x_bounds([-1.2, 1.2])
        .y_bounds([-1.2, 1.2])
        .paint(move |ctx| {
            use ratatui::widgets::canvas::{Circle, Points};
            use ratatui::style::Color;

            // Draw elevation circles (90°, 60°, 30°, horizon)
            let circles = [
                (0.0, Color::DarkGray),   // Center (zenith)
                (0.33, Color::DarkGray),  // 60°
                (0.66, Color::DarkGray),  // 30°
                (1.0, Color::Gray),       // Horizon
            ];

            for (radius, color) in circles.iter() {
                let mut points = Vec::new();
                for angle in (0..360).step_by(5) {
                    let rad = (angle as f64) * PI / 180.0;
                    let x = radius * rad.cos();
                    let y = radius * rad.sin();
                    points.push((x, y));
                }
                ctx.draw(&Points {
                    coords: &points,
                    color: *color,
                });
            }

            // Draw compass directions using circles at the positions
            ctx.draw(&Circle {
                x: 0.0,
                y: 1.05,
                radius: 0.02,
                color: Color::White,
            });
            ctx.draw(&Circle {
                x: 0.0,
                y: -1.05,
                radius: 0.02,
                color: Color::White,
            });
            ctx.draw(&Circle {
                x: 1.05,
                y: 0.0,
                radius: 0.02,
                color: Color::White,
            });
            ctx.draw(&Circle {
                x: -1.05,
                y: 0.0,
                radius: 0.02,
                color: Color::White,
            });

            // Plot satellites
            for sat in satellites.values() {
                if let (Some(elevation), Some(azimuth)) = (sat.elevation, sat.azimuth) {
                    // Convert elevation and azimuth to x, y coordinates
                    let radius = 1.0 - (elevation as f64 / 90.0);
                    let azimuth_rad = (azimuth as f64 - 90.0) * PI / 180.0;
                    
                    let x = radius * azimuth_rad.cos();
                    let y = radius * azimuth_rad.sin();

                    let color = match sat.system {
                        GnssSystem::Gps => Color::Green,
                        GnssSystem::Glonass => Color::Blue,
                        GnssSystem::Galileo => Color::Cyan,
                        GnssSystem::Beidou => Color::Magenta,
                        GnssSystem::Unknown => Color::Gray,
                    };

                    // Use different circle sizes based on SNR
                    let radius = if let Some(snr) = sat.snr {
                        if snr >= 40 {
                            0.05 // Large for strong signal
                        } else if snr >= 30 {
                            0.03 // Medium
                        } else {
                            0.02 // Small for weak
                        }
                    } else {
                        0.01 // Tiny for no signal
                    };

                    ctx.draw(&Circle {
                        x,
                        y,
                        radius,
                        color,
                    });
                }
            }
        })
}

/// Create telemetry widget showing racing metrics
pub fn create_telemetry_widget(gps_data: &GpsData) -> Paragraph<'static> {
    // Show available data from navigation
    let speed_knots = gps_data
        .navigation
        .speed_knots
        .map(|v| format!("{:.1} kts", v))
        .unwrap_or_else(|| "N/A".to_string());

    let heading_rate = gps_data
        .navigation
        .heading_rate
        .map(|v| format!("{:.1}°/s", v))
        .unwrap_or_else(|| "N/A".to_string());

    let true_heading = gps_data
        .navigation
        .true_heading
        .map(|v| format!("{:.1}°", v))
        .unwrap_or_else(|| "N/A".to_string());

    let accuracy = gps_data
        .navigation
        .position_accuracy
        .map(|v| format!("{:.1} m", v))
        .unwrap_or_else(|| "N/A".to_string());

    let text = vec![
        Line::from(vec![
            Span::styled("Speed (kts):", Style::default().fg(Color::Cyan)),
            Span::raw(format!(" {}", speed_knots)),
        ]),
        Line::from(vec![
            Span::styled("Head Rate: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!(" {}", heading_rate)),
        ]),
        Line::from(vec![
            Span::styled("True Head: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!(" {}", true_heading)),
        ]),
        Line::from(vec![
            Span::styled("Accuracy:  ", Style::default().fg(Color::Cyan)),
            Span::raw(format!(" {}", accuracy)),
        ]),
    ];

    Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("📊 Advanced Data"),
    )
}

/// Create connection status widget
pub fn create_connection_widget(state: &crate::models::AppState) -> Paragraph<'static> {
    use crate::models::MqttStatus;

    let mqtt_status = match state.mqtt_status {
        MqttStatus::Connected => ("✓ Connected", Color::Green),
        MqttStatus::Connecting => ("⟳ Connecting", Color::Yellow),
        MqttStatus::Disconnected => ("✗ Disconnected", Color::Red),
        MqttStatus::Error => ("✗ Error", Color::Red),
    };

    let serial_status = if state.serial_connected {
        ("✓ Connected", Color::Green)
    } else {
        ("✗ Disconnected", Color::Red)
    };

    let text = vec![
        Line::from(vec![
            Span::styled("MQTT:   ", Style::default().fg(Color::Cyan)),
            Span::styled(mqtt_status.0, Style::default().fg(mqtt_status.1)),
        ]),
        Line::from(vec![
            Span::styled("Serial: ", Style::default().fg(Color::Cyan)),
            Span::styled(serial_status.0, Style::default().fg(serial_status.1)),
        ]),
    ];

    Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("🔌 Connections"),
    )
}
