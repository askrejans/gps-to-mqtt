use crate::models::{AppState, GnssSystem, GpsData, MqttStatus};
use ratatui::widgets::canvas::Canvas;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
};
use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Style helpers (speeduino-style)
// ---------------------------------------------------------------------------

pub fn section_line(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("── {} ──", title),
        Style::default().fg(Color::DarkGray),
    ))
}

pub fn data_row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<12}", label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(value),
    ])
}

pub fn status_indicator(connected: bool) -> Span<'static> {
    if connected {
        Span::styled("● ONLINE ", Style::default().fg(Color::Green))
    } else {
        Span::styled("○ OFFLINE", Style::default().fg(Color::Red))
    }
}

// ---------------------------------------------------------------------------
// Connections panel
// ---------------------------------------------------------------------------

/// Renders the connections panel (left column of Overview tab).
pub fn render_connections_widget(state: &AppState) -> Paragraph<'static> {
    let mqtt_connected = state.mqtt_status == MqttStatus::Connected;
    let mut lines: Vec<Line> = Vec::new();

    // GPS / serial connection
    lines.push(Line::from(vec![
        Span::styled("GPS:  ", Style::default().add_modifier(Modifier::BOLD)),
        status_indicator(state.serial_connected),
    ]));
    if !state.connection_address.is_empty() {
        lines.push(Line::from(Span::raw(format!(
            "  {}",
            state.connection_address
        ))));
    }
    lines.push(Line::default());

    // MQTT connection
    lines.push(Line::from(vec![
        Span::styled("MQTT: ", Style::default().add_modifier(Modifier::BOLD)),
        if state.mqtt_enabled {
            status_indicator(mqtt_connected)
        } else {
            Span::styled("DISABLED ", Style::default().fg(Color::DarkGray))
        },
    ]));
    if state.mqtt_enabled && !state.mqtt_address.is_empty() {
        lines.push(Line::from(Span::raw(format!("  {}", state.mqtt_address))));
    }
    lines.push(Line::default());

    if state.mqtt_enabled {
        lines.push(Line::from(vec![
            Span::styled("Msgs: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(state.messages_published.to_string()),
        ]));
    }

    let block = Block::default()
        .title(" CONNECTIONS ")
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1));
    Paragraph::new(lines).block(block).wrap(Wrap { trim: true })
}

// ---------------------------------------------------------------------------
// GPS data panel
// ---------------------------------------------------------------------------

/// Renders the main GPS data panel with sections (right column of Overview tab).
pub fn render_gps_data_widget(gps_data: &GpsData) -> Paragraph<'static> {
    let block = Block::default()
        .title(" GPS DATA ")
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1));

    let mut lines: Vec<Line> = Vec::new();

    // ── POSITION ─────────────────────────────────────────────────────────
    lines.push(section_line("POSITION"));
    lines.push(data_row(
        "Latitude",
        gps_data
            .navigation
            .latitude
            .map(|v| format!("{:.6}°", v))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Longitude",
        gps_data
            .navigation
            .longitude
            .map(|v| format!("{:.6}°", v))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Altitude",
        gps_data
            .navigation
            .altitude
            .map(|v| format!("{:.1} m", v))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Speed",
        gps_data
            .navigation
            .speed_kph
            .map(|v| format!("{:.1} km/h", v))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Course",
        gps_data
            .navigation
            .course
            .map(|v| format!("{:.1}°", v))
            .unwrap_or_else(|| "—".into()),
    ));

    // ── FIX ──────────────────────────────────────────────────────────────
    lines.push(Line::default());
    lines.push(section_line("FIX"));
    lines.push(data_row(
        "Fix type",
        gps_data
            .fix
            .fix_type
            .as_ref()
            .map(|f| format!("{:?}", f))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Quality",
        gps_data
            .fix
            .fix_quality
            .as_ref()
            .map(|q| format!("{:?}", q))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Sats used",
        gps_data
            .fix
            .satellites_used
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Sats view",
        gps_data
            .satellites_in_view
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "HDOP",
        gps_data
            .fix
            .hdop
            .map(|h| format!("{:.2}", h))
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Time (UTC)",
        gps_data
            .fix
            .time
            .map(|t| t.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "—".into()),
    ));
    lines.push(data_row(
        "Date",
        gps_data
            .fix
            .date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "—".into()),
    ));

    // ── HEADING / ACCURACY ───────────────────────────────────────────────
    if gps_data.navigation.true_heading.is_some()
        || gps_data.navigation.heading_rate.is_some()
        || gps_data.navigation.position_accuracy.is_some()
    {
        lines.push(Line::default());
        lines.push(section_line("HEADING / ACCURACY"));
        if let Some(h) = gps_data.navigation.true_heading {
            lines.push(data_row("True hdg", format!("{:.1}°", h)));
        }
        if let Some(r) = gps_data.navigation.heading_rate {
            lines.push(data_row("Hdg rate", format!("{:.1}°/s", r)));
        }
        if let Some(a) = gps_data.navigation.position_accuracy {
            lines.push(data_row("Pos acc", format!("{:.1} m", a)));
        }
    }

    Paragraph::new(lines).block(block)
}

// ---------------------------------------------------------------------------
// Satellite widgets (preserved, lightly restyled)
// ---------------------------------------------------------------------------

/// Create satellite list widget
pub fn create_satellite_list_widget(gps_data: &GpsData) -> List<'static> {
    let satellites_by_system = gps_data.satellites_by_system();
    let mut items = Vec::new();

    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            "PRN  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "El   ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Az   ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "SNR  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Sys",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])));

    for (system, sats) in satellites_by_system.iter() {
        let system_color = match system {
            GnssSystem::Gps => Color::Green,
            GnssSystem::Glonass => Color::Blue,
            GnssSystem::Galileo => Color::Cyan,
            GnssSystem::Beidou => Color::Magenta,
            GnssSystem::Unknown => Color::Gray,
        };

        for sat in sats.iter() {
            let prn = format!("{:<5}", sat.prn);
            let el = sat
                .elevation
                .map(|e| format!("{:<5}", e))
                .unwrap_or_else(|| "—    ".into());
            let az = sat
                .azimuth
                .map(|a| format!("{:<5}", a))
                .unwrap_or_else(|| "—    ".into());
            let snr = sat
                .snr
                .map(|s| format!("{:<5}", s))
                .unwrap_or_else(|| "—    ".into());
            let snr_color = sat
                .snr
                .map(|s| {
                    if s >= 40 {
                        Color::Green
                    } else if s >= 30 {
                        Color::Yellow
                    } else {
                        Color::Red
                    }
                })
                .unwrap_or(Color::Gray);

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
        items.push(ListItem::new(Line::from(Span::styled(
            "No satellites in view",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    List::new(items).block(
        Block::default()
            .title(" SATELLITE DETAILS ")
            .borders(Borders::ALL),
    )
}

/// Create satellite sky chart widget
pub fn create_satellite_sky_chart(
    gps_data: &GpsData,
) -> Canvas<'static, impl Fn(&mut ratatui::widgets::canvas::Context) + 'static> {
    let satellites = gps_data.satellites.clone();

    Canvas::default()
        .block(Block::default().borders(Borders::ALL).title(" SKY VIEW "))
        .x_bounds([-1.2, 1.2])
        .y_bounds([-1.2, 1.2])
        .paint(move |ctx| {
            use ratatui::widgets::canvas::{Circle, Points};

            // Draw elevation rings
            for (radius, color) in &[
                (0.0_f64, Color::DarkGray),
                (0.33, Color::DarkGray),
                (0.66, Color::DarkGray),
                (1.0, Color::Gray),
            ] {
                let pts: Vec<(f64, f64)> = (0..360)
                    .step_by(5)
                    .map(|angle| {
                        let rad = (angle as f64) * PI / 180.0;
                        (radius * rad.cos(), radius * rad.sin())
                    })
                    .collect();
                ctx.draw(&Points {
                    coords: &pts,
                    color: *color,
                });
            }

            // Cardinal direction dots
            for (x, y) in &[(0.0, 1.05), (0.0, -1.05), (1.05, 0.0), (-1.05, 0.0)] {
                ctx.draw(&Circle {
                    x: *x,
                    y: *y,
                    radius: 0.02,
                    color: Color::White,
                });
            }

            // Plot satellites
            for sat in satellites.values() {
                if let (Some(elevation), Some(azimuth)) = (sat.elevation, sat.azimuth) {
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
                    let r = sat
                        .snr
                        .map(|s| {
                            if s >= 40 {
                                0.05
                            } else if s >= 30 {
                                0.03
                            } else {
                                0.02
                            }
                        })
                        .unwrap_or(0.01);
                    ctx.draw(&Circle {
                        x,
                        y,
                        radius: r,
                        color,
                    });
                }
            }
        })
}
