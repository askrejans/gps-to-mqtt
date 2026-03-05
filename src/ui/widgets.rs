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
            Span::raw(
                state
                    .messages_published
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .to_string(),
            ),
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
// Satellite widgets
// ---------------------------------------------------------------------------

fn system_prefix_and_color(system: &GnssSystem) -> (&'static str, Color) {
    match system {
        GnssSystem::Gps => ("G", Color::Green),
        GnssSystem::Glonass => ("R", Color::Blue),
        GnssSystem::Galileo => ("E", Color::Cyan),
        GnssSystem::Beidou => ("B", Color::Magenta),
        GnssSystem::Unknown => ("?", Color::Gray),
    }
}

/// Create satellite detail list (left panel of Satellites tab).
pub fn create_satellite_list_widget(gps_data: &GpsData) -> List<'static> {
    let by_sys = gps_data.satellites_by_system();
    let total: usize = by_sys.values().map(|v| v.len()).sum();
    let mut items: Vec<ListItem> = Vec::new();

    // ── Summary row ───────────────────────────────────────────────────────────
    let mut summary_spans = vec![Span::styled(
        format!("{} tracked  ", total),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    for (sys, sats) in &by_sys {
        let (pfx, col) = system_prefix_and_color(sys);
        summary_spans.push(Span::styled(
            format!("{}:{} ", pfx, sats.len()),
            Style::default().fg(col).add_modifier(Modifier::BOLD),
        ));
    }
    items.push(ListItem::new(Line::from(summary_spans)));

    // ── Column header ─────────────────────────────────────────────────────────
    items.push(ListItem::new(Line::from(Span::styled(
        "─────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    ))));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            " ID   ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " El  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "    Az  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Signal (dB)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])));

    // ── Per-constellation groups ───────────────────────────────────────────────
    for (system, sats) in by_sys.iter() {
        let (pfx, sys_col) = system_prefix_and_color(system);
        let sys_name = match system {
            GnssSystem::Gps => "GPS",
            GnssSystem::Glonass => "GLONASS",
            GnssSystem::Galileo => "GALILEO",
            GnssSystem::Beidou => "BEIDOU",
            GnssSystem::Unknown => "OTHER",
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(" ▸ {} ", sys_name),
                Style::default().fg(sys_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({} sats)", sats.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ])));

        for sat in sats.iter() {
            let id = format!("{}{:02}", pfx, sat.prn % 100);
            let el_str = sat
                .elevation
                .map(|e| format!("{:3}°", e))
                .unwrap_or_else(|| "  — ".to_string());
            let az_str = sat
                .azimuth
                .map(|a| format!("{:4}°", a))
                .unwrap_or_else(|| "   — ".to_string());
            let (bar, val_str, snr_col) = match sat.snr {
                Some(s) => {
                    let filled = ((s.min(50).max(0) as usize) * 8) / 50;
                    let bar_str = format!("{}{}", "█".repeat(filled), "░".repeat(8 - filled));
                    let col = if s >= 40 {
                        Color::Green
                    } else if s >= 25 {
                        Color::Yellow
                    } else {
                        Color::Red
                    };
                    (bar_str, format!("{:2}", s), col)
                }
                None => ("░░░░░░░░".to_string(), " —".to_string(), Color::DarkGray),
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {:4}", id), Style::default().fg(sys_col)),
                Span::styled(format!("  {} ", el_str), Style::default().fg(Color::White)),
                Span::styled(format!("  {} ", az_str), Style::default().fg(Color::White)),
                Span::styled(bar, Style::default().fg(snr_col)),
                Span::styled(format!(" {}", val_str), Style::default().fg(snr_col)),
            ])));
        }
        items.push(ListItem::new(Line::default())); // spacer between constellations
    }

    if total == 0 {
        items.push(ListItem::new(Line::from(Span::styled(
            "  Waiting for satellites...",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    // ── Legend ────────────────────────────────────────────────────────────────
    items.push(ListItem::new(Line::from(Span::styled(
        "─────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    ))));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            " G ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("GPS  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "R ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Glonass  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "E ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Galileo  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "B ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("BeiDou", Style::default().fg(Color::DarkGray)),
    ])));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(" ●", Style::default().fg(Color::Green)),
        Span::styled(" ≥40 dB  ", Style::default().fg(Color::DarkGray)),
        Span::styled("●", Style::default().fg(Color::Yellow)),
        Span::styled(" ≥25 dB  ", Style::default().fg(Color::DarkGray)),
        Span::styled("●", Style::default().fg(Color::Red)),
        Span::styled(" <25 dB  ", Style::default().fg(Color::DarkGray)),
        Span::styled("●", Style::default().fg(Color::DarkGray)),
        Span::styled(" no signal", Style::default().fg(Color::DarkGray)),
    ])));

    List::new(items).block(
        Block::default()
            .title(" SATELLITE DETAILS ")
            .borders(Borders::ALL),
    )
}

/// Create satellite sky chart (right panel of Satellites tab).
///
/// North is at the top, East to the right (standard sky-view orientation).
/// Concentric rings mark 0° (horizon), 30°, and 60° elevation.
/// Each satellite is plotted by its azimuth/elevation with its PRN label.
/// Dot colour indicates signal quality; label colour indicates GNSS system.
pub fn create_satellite_sky_chart(
    gps_data: &GpsData,
) -> Canvas<'static, impl Fn(&mut ratatui::widgets::canvas::Context) + 'static> {
    let sats: Vec<_> = gps_data.satellites.values().cloned().collect();

    Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SKY VIEW  (N↑  E→) "),
        )
        .x_bounds([-1.35, 1.35])
        .y_bounds([-1.35, 1.35])
        .paint(move |ctx| {
            use ratatui::widgets::canvas::{Circle, Points};

            // ── N/S and E/W axis guide lines ──────────────────────────────────
            let ns: Vec<(f64, f64)> = (-13..=13).map(|i| (0.0_f64, i as f64 * 0.1)).collect();
            let ew: Vec<(f64, f64)> = (-13..=13).map(|i| (i as f64 * 0.1, 0.0_f64)).collect();
            ctx.draw(&Points {
                coords: &ns,
                color: Color::DarkGray,
            });
            ctx.draw(&Points {
                coords: &ew,
                color: Color::DarkGray,
            });

            // ── Elevation rings ───────────────────────────────────────────────
            // r=1.0 → horizon (0°),  r=0.667 → 30°,  r=0.333 → 60°
            for &(radius, step, label, ring_col) in &[
                (1.000_f64, 3_usize, "0° ", Color::Gray),
                (0.667_f64, 5_usize, "30°", Color::DarkGray),
                (0.333_f64, 8_usize, "60°", Color::DarkGray),
            ] {
                let pts: Vec<(f64, f64)> = (0..360_usize)
                    .step_by(step)
                    .map(|a| {
                        let rad = a as f64 * PI / 180.0;
                        (radius * rad.cos(), radius * rad.sin())
                    })
                    .collect();
                ctx.draw(&Points {
                    coords: &pts,
                    color: ring_col,
                });
                // Elevation label on the East side of each ring, just inside
                ctx.print(
                    radius - 0.03,
                    0.09,
                    Line::from(Span::styled(
                        label.to_string(),
                        Style::default().fg(Color::DarkGray),
                    )),
                );
            }

            // Centre marker = zenith (90°)
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: 0.03,
                color: Color::DarkGray,
            });
            ctx.print(
                0.05,
                0.07,
                Line::from(Span::styled(
                    "90°".to_string(),
                    Style::default().fg(Color::DarkGray),
                )),
            );

            // ── Cardinal direction labels ─────────────────────────────────────
            for &(lx, ly, label) in &[
                (0.0_f64, 1.12_f64, "N"),
                (0.0_f64, -1.28_f64, "S"),
                (1.12_f64, -0.04_f64, "E"),
                (-1.28_f64, -0.04_f64, "W"),
            ] {
                ctx.print(
                    lx,
                    ly,
                    Line::from(Span::styled(
                        label.to_string(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                );
            }

            // ── Satellite dots + PRN labels ───────────────────────────────────
            for sat in &sats {
                let (Some(el), Some(az)) = (sat.elevation, sat.azimuth) else {
                    continue;
                };

                // Radial distance: 1.0 at horizon (el=0°), 0.0 at zenith (el=90°)
                let r = 1.0 - (el as f64 / 90.0);

                // North-up polar: az=0°→top, az=90°→right, az=180°→bottom, az=270°→left
                let az_rad = (az as f64) * PI / 180.0;
                let x = r * az_rad.sin();
                let y = r * az_rad.cos();

                let (pfx, sys_col) = match sat.system {
                    GnssSystem::Gps => ("G", Color::Green),
                    GnssSystem::Glonass => ("R", Color::Blue),
                    GnssSystem::Galileo => ("E", Color::Cyan),
                    GnssSystem::Beidou => ("B", Color::Magenta),
                    GnssSystem::Unknown => ("?", Color::Gray),
                };

                // Dot colour: bright system colour if strong, yellow if ok, red if weak
                let dot_col = match sat.snr {
                    Some(s) if s >= 40 => sys_col,
                    Some(s) if s >= 25 => Color::Yellow,
                    Some(_) => Color::Red,
                    None => Color::DarkGray,
                };
                let dot_r = if sat.snr.map(|s| s >= 35).unwrap_or(false) {
                    0.05
                } else {
                    0.03
                };

                ctx.draw(&Circle {
                    x,
                    y,
                    radius: dot_r,
                    color: dot_col,
                });

                // PRN label offset just above-right of the dot
                let label = format!("{}{:02}", pfx, sat.prn % 100);
                ctx.print(
                    x + 0.06,
                    y + 0.04,
                    Line::from(Span::styled(label, Style::default().fg(sys_col))),
                );
            }
        })
}
