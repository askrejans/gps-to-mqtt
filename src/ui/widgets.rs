//! Panels for the TUI. Pure functions of [`View`] so they are easy to test.

use super::View;
use crate::models::{
    Constellation, Fix, FixMode, FixQuality, M_TO_FT, MPS_TO_KNOTS, MPS_TO_KPH, MPS_TO_MPH,
};
use crate::shared::LinkStatus;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, Padding, Paragraph, Row, Table,
        canvas::{Canvas, Circle, Line as CLine, Points},
    },
};
use std::f64::consts::PI;

const DASH: &str = "—";

fn label(s: &str) -> Span<'static> {
    Span::styled(format!("{s:<12}"), Style::default().fg(Color::Cyan))
}

fn row(name: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![label(name), Span::raw(value.into())])
}

fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> String) -> String {
    v.map(f).unwrap_or_else(|| DASH.into())
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
}

fn link(status: LinkStatus) -> Span<'static> {
    match status {
        LinkStatus::Connected => Span::styled("●", Style::default().fg(Color::Green)),
        LinkStatus::Connecting => Span::styled("◌", Style::default().fg(Color::Yellow)),
        LinkStatus::Disconnected => Span::styled("○", Style::default().fg(Color::Red)),
        LinkStatus::Disabled => Span::styled("–", Style::default().fg(Color::DarkGray)),
    }
}

/// Short fix badge: text + colour.
pub fn fix_badge(fix: Option<&Fix>, age: Option<f64>) -> (String, Color) {
    let stale = age.is_none_or(|a| a > 3.0);
    match fix {
        None => ("NO DATA".into(), Color::DarkGray),
        Some(f) if !f.valid => ("NO FIX".into(), Color::Red),
        Some(_) if stale => ("STALE".into(), Color::Yellow),
        Some(f) => match (f.quality, f.mode) {
            (FixQuality::Rtk, _) => ("RTK FIXED".into(), Color::Magenta),
            (FixQuality::FloatRtk, _) => ("RTK FLOAT".into(), Color::Magenta),
            (FixQuality::Dgps, FixMode::Fix3D) => ("3D DGPS".into(), Color::Green),
            (FixQuality::Estimated, _) => ("DEAD RECK.".into(), Color::Yellow),
            (_, FixMode::Fix3D) => ("3D FIX".into(), Color::Green),
            (_, FixMode::Fix2D) => ("2D FIX".into(), Color::LightGreen),
            _ => ("FIX".into(), Color::Green),
        },
    }
}

pub fn header(v: &View) -> Paragraph<'static> {
    let (badge, color) = fix_badge(v.fix.as_deref(), v.fix_age);
    let sep = || Span::styled(" │ ", Style::default().fg(Color::DarkGray));
    let detail = if v.source_detail.is_empty() {
        v.info.source.clone()
    } else {
        v.source_detail.clone()
    };
    let mut spans = vec![
        Span::styled(
            format!(" gps-to-mqtt {}", v.info.version),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        sep(),
        link(v.source),
        Span::raw(format!(" {detail}")),
        sep(),
        Span::styled(
            format!(" {badge} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(opt(v.fix_age, |a| format!(" {a:.1} s"))),
        sep(),
        Span::raw("MQTT "),
        link(v.mqtt),
        sep(),
        Span::raw(format!(
            "{:.0} sent/s · {:.0} fix/s",
            v.sentence_rate, v.fix_rate
        )),
    ];
    if v.replay {
        spans.push(sep());
        spans.push(Span::styled("REPLAY", Style::default().fg(Color::Magenta)));
    }
    Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL))
}

fn dms(deg: f64, pos: char, neg: char) -> String {
    let h = if deg >= 0.0 { pos } else { neg };
    let d = deg.abs();
    let m = (d.fract()) * 60.0;
    format!(
        "{}°{:02}'{:06.3}\"{h}",
        d.trunc(),
        m.trunc(),
        m.fract() * 60.0
    )
}

pub fn overview(f: &mut Frame, area: Rect, v: &View) {
    let [left, mid, right] = Layout::horizontal([
        Constraint::Percentage(36),
        Constraint::Percentage(30),
        Constraint::Percentage(34),
    ])
    .areas(area);
    let fix = v.fix.as_deref();
    let pos = fix.and_then(Fix::position);

    let speed = fix.and_then(|f| f.speed_mps);
    let mut lines = vec![
        row(
            "Latitude",
            opt(pos, |(lat, _)| {
                format!("{lat:>12.7}°  {}", dms(lat, 'N', 'S'))
            }),
        ),
        row(
            "Longitude",
            opt(pos, |(_, lon)| {
                format!("{lon:>12.7}°  {}", dms(lon, 'E', 'W'))
            }),
        ),
        row(
            "MGRS",
            opt(fix.filter(|f| f.valid).and_then(|f| f.mgrs.clone()), |m| m),
        ),
        row(
            "Altitude",
            opt(fix.and_then(|f| f.altitude_m), |a| {
                format!("{a:.1} m  ({:.0} ft)", a * M_TO_FT)
            }),
        ),
        Line::default(),
        Line::from(vec![
            label("Speed"),
            Span::styled(
                opt(speed, |s| format!("{:.1} km/h", s * MPS_TO_KPH)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        row(
            "",
            opt(speed, |s| {
                format!(
                    "{:.1} mph · {:.1} kn · {:.2} m/s",
                    s * MPS_TO_MPH,
                    s * MPS_TO_KNOTS,
                    s
                )
            }),
        ),
        row(
            "Course",
            opt(fix.and_then(|f| f.course_deg), |c| format!("{c:.1}°")),
        ),
    ];
    if let Some(h) = fix.and_then(|f| f.heading_deg) {
        lines.push(row("Heading", format!("{h:.1}° true")));
    }
    if let Some(r) = fix.and_then(|f| f.rate_of_turn_dps) {
        lines.push(row("Rate of turn", format!("{r:+.2}°/s")));
    }
    f.render_widget(Paragraph::new(lines).block(panel(" POSITION ")), left);

    let acc = fix.and_then(|f| f.accuracy_m.zip(f.accuracy_source));
    let lines = vec![
        row("Quality", opt(fix, |f| f.quality.label().to_string())),
        row(
            "Mode",
            opt(fix, |f| match f.mode {
                FixMode::NoFix => "none".into(),
                FixMode::Fix2D => "2D".into(),
                FixMode::Fix3D => "3D".into(),
            }),
        ),
        row(
            "Satellites",
            format!(
                "{} used / {} view",
                opt(fix.and_then(|f| f.satellites_used), |n| n.to_string()),
                v.sky.in_view
            ),
        ),
        row(
            "DOP h/v/p",
            format!(
                "{} / {} / {}",
                opt(fix.and_then(|f| f.hdop), |x| format!("{x:.2}")),
                opt(fix.and_then(|f| f.vdop), |x| format!("{x:.2}")),
                opt(fix.and_then(|f| f.pdop), |x| format!("{x:.2}")),
            ),
        ),
        row(
            "Accuracy",
            opt(acc, |(a, src)| {
                let src = serde_json::to_value(src)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                format!("±{a:.1} m ({src})")
            }),
        ),
        row(
            "UTC",
            opt(fix.and_then(|f| f.time), |t| {
                t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
            }),
        ),
        row("Sequence", opt(fix, |f| f.sequence.to_string())),
        row("Receiver", v.info.receiver.clone()),
    ];
    f.render_widget(Paragraph::new(lines).block(panel(" FIX ")), mid);

    dynamics(f, right, fix);
}

/// g-g diagram (friction circle) plus numeric dynamics.
fn dynamics(f: &mut Frame, area: Rect, fix: Option<&Fix>) {
    let t = fix.and_then(|f| f.telemetry.clone());
    let [plot, text] = Layout::vertical([Constraint::Min(6), Constraint::Length(5)]).areas(area);
    let (lat_g, long_g) = t
        .as_ref()
        .map(|t| (t.lateral_g.unwrap_or(0.0), t.longitudinal_g.unwrap_or(0.0)))
        .unwrap_or((0.0, 0.0));
    let braking = t.as_ref().is_some_and(|t| t.braking);
    let canvas = Canvas::default()
        .block(
            Block::default()
                .title(" DYNAMICS  g-g ")
                .borders(Borders::ALL),
        )
        .marker(Marker::Braille)
        .x_bounds([-1.6, 1.6])
        .y_bounds([-1.6, 1.6])
        .paint(move |ctx| {
            for r in [0.5, 1.0, 1.5] {
                ctx.draw(&Circle {
                    x: 0.0,
                    y: 0.0,
                    radius: r,
                    color: Color::DarkGray,
                });
            }
            ctx.draw(&CLine {
                x1: -1.6,
                y1: 0.0,
                x2: 1.6,
                y2: 0.0,
                color: Color::DarkGray,
            });
            ctx.draw(&CLine {
                x1: 0.0,
                y1: -1.6,
                x2: 0.0,
                y2: 1.6,
                color: Color::DarkGray,
            });
            ctx.print(
                1.02,
                0.05,
                Span::styled("1g", Style::default().fg(Color::DarkGray)),
            );
            let x = lat_g.clamp(-1.6, 1.6);
            let y = long_g.clamp(-1.6, 1.6);
            let color = if braking { Color::Red } else { Color::Green };
            ctx.draw(&CLine {
                x1: 0.0,
                y1: 0.0,
                x2: x,
                y2: y,
                color,
            });
            ctx.draw(&Circle {
                x,
                y,
                radius: 0.06,
                color,
            });
        });
    f.render_widget(canvas, plot);

    let fmt = |x: Option<f64>, unit: &str| opt(x, |x| format!("{x:+.2}{unit}"));
    let lines = match &t {
        Some(t) => vec![
            row(
                "Long / Lat",
                format!(
                    "{} / {}",
                    fmt(t.longitudinal_g, " g"),
                    fmt(t.lateral_g, " g")
                ),
            ),
            row("Combined", opt(t.combined_g, |g| format!("{g:.2} g"))),
            row("Yaw rate", fmt(t.yaw_rate_dps, "°/s")),
            Line::from(vec![
                label("Trip"),
                Span::raw(format!(
                    "{:.2} km · max {:.0} km/h ",
                    t.distance_m / 1000.0,
                    t.max_speed_mps * MPS_TO_KPH
                )),
                if t.braking {
                    Span::styled("BRAKE", Style::default().fg(Color::Black).bg(Color::Red))
                } else {
                    Span::raw("")
                },
            ]),
        ],
        None => vec![Line::styled(
            "telemetry disabled or no fix",
            Style::default().fg(Color::DarkGray),
        )],
    };
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .padding(Padding::horizontal(1)),
        ),
        text,
    );
}

fn constellation_color(c: Constellation) -> Color {
    match c {
        Constellation::Gps => Color::Green,
        Constellation::Glonass => Color::Blue,
        Constellation::Galileo => Color::Cyan,
        Constellation::Beidou => Color::Magenta,
        Constellation::Qzss => Color::LightRed,
        Constellation::Sbas => Color::Yellow,
        Constellation::Navic => Color::LightBlue,
        Constellation::Unknown => Color::Gray,
    }
}

fn snr_color(snr: Option<i32>) -> Color {
    match snr {
        Some(s) if s >= 40 => Color::Green,
        Some(s) if s >= 25 => Color::Yellow,
        Some(_) => Color::Red,
        None => Color::DarkGray,
    }
}

pub fn satellites(f: &mut Frame, area: Rect, v: &View) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);

    let mut summary = vec![Span::styled(
        format!("{} in view · {} used  ", v.sky.in_view, v.sky.used),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    let mut counts: Vec<(Constellation, usize)> = Vec::new();
    for s in &v.sky.satellites {
        match counts.iter_mut().find(|(c, _)| *c == s.constellation) {
            Some((_, n)) => *n += 1,
            None => counts.push((s.constellation, 1)),
        }
    }
    for (c, n) in counts {
        summary.push(Span::styled(
            format!("{}:{n} ", c.letter()),
            Style::default().fg(constellation_color(c)),
        ));
    }

    let rows = v.sky.satellites.iter().map(|s| {
        let snr = s.snr_dbhz;
        let filled = snr.map_or(0, |x| (x.clamp(0, 50) as usize * 10) / 50);
        Row::new(vec![
            Span::styled(
                s.id(),
                Style::default().fg(constellation_color(s.constellation)),
            ),
            Span::raw(s.constellation.name()),
            Span::raw(opt(s.elevation_deg, |e| format!("{e:>3}°"))),
            Span::raw(opt(s.azimuth_deg, |a| format!("{a:>3}°"))),
            Span::styled(
                format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled)),
                Style::default().fg(snr_color(snr)),
            ),
            Span::styled(
                opt(snr, |x| format!("{x:>2}")),
                Style::default().fg(snr_color(snr)),
            ),
            if s.used {
                Span::styled("●", Style::default().fg(Color::Green))
            } else {
                Span::styled("○", Style::default().fg(Color::DarkGray))
            },
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(3),
            Constraint::Length(4),
        ],
    )
    .header(
        Row::new(["ID", "System", "El", "Az", "C/N₀ dB-Hz", "", "Used"])
            .style(Style::default().fg(Color::Yellow)),
    )
    .block(
        Block::default()
            .title(Line::from(summary))
            .borders(Borders::ALL),
    );
    f.render_widget(table, left);

    let sats = v.sky.satellites.clone();
    let sky = Canvas::default()
        .block(
            Block::default()
                .title(" SKY VIEW (N↑ E→) ")
                .borders(Borders::ALL),
        )
        .marker(Marker::Braille)
        .x_bounds([-1.3, 1.3])
        .y_bounds([-1.3, 1.3])
        .paint(move |ctx| {
            for (r, txt) in [(1.0, "0°"), (2.0 / 3.0, "30°"), (1.0 / 3.0, "60°")] {
                ctx.draw(&Circle {
                    x: 0.0,
                    y: 0.0,
                    radius: r,
                    color: Color::DarkGray,
                });
                ctx.print(
                    r * 0.72,
                    r * 0.72,
                    Span::styled(txt, Style::default().fg(Color::DarkGray)),
                );
            }
            ctx.draw(&Points {
                coords: &[(0.0, 0.0)],
                color: Color::DarkGray,
            });
            for (x, y, t) in [
                (0.0, 1.12, "N"),
                (0.0, -1.22, "S"),
                (1.12, 0.0, "E"),
                (-1.22, 0.0, "W"),
            ] {
                ctx.print(
                    x,
                    y,
                    Span::styled(
                        t,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
            }
            for s in &sats {
                let (Some(el), Some(az)) = (s.elevation_deg, s.azimuth_deg) else {
                    continue;
                };
                let r = 1.0 - (el.clamp(0, 90) as f64 / 90.0);
                let a = az as f64 * PI / 180.0;
                let (x, y) = (r * a.sin(), r * a.cos());
                let color = if s.used {
                    constellation_color(s.constellation)
                } else {
                    snr_color(s.snr_dbhz)
                };
                ctx.draw(&Circle {
                    x,
                    y,
                    radius: if s.used { 0.05 } else { 0.03 },
                    color,
                });
                ctx.print(
                    x + 0.06,
                    y + 0.03,
                    Span::styled(
                        s.id(),
                        Style::default().fg(constellation_color(s.constellation)),
                    ),
                );
            }
        });
    f.render_widget(sky, right);
}

pub fn stats(f: &mut Frame, area: Rect, v: &View) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let mut lines: Vec<Line> = v
        .counters
        .iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(format!("{name:<18}"), Style::default().fg(Color::Cyan)),
                Span::raw(value.to_string()),
            ])
        })
        .collect();
    lines.push(Line::default());
    let up = v.uptime.as_secs();
    lines.push(row(
        "Uptime",
        format!("{}h {:02}m {:02}s", up / 3600, up / 60 % 60, up % 60),
    ));
    f.render_widget(Paragraph::new(lines).block(panel(" COUNTERS ")), left);

    let mut items: Vec<ListItem> = vec![ListItem::new(row("Source", v.info.source.clone()))];
    items.push(ListItem::new(row("Receiver", v.info.receiver.clone())));
    items.push(ListItem::new(Line::default()));
    items.push(ListItem::new(Line::styled(
        "Outputs",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if v.info.outputs.is_empty() {
        items.push(ListItem::new(Line::styled(
            "  none (display only)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for o in &v.info.outputs {
        items.push(ListItem::new(format!("  {o}")));
    }
    items.push(ListItem::new(Line::default()));
    items.push(ListItem::new(Line::styled(
        "Receiver messages (TXT)",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for m in v
        .messages
        .iter()
        .rev()
        .take(area.height.saturating_sub(10) as usize)
    {
        items.push(ListItem::new(Line::styled(
            format!("  {m}"),
            Style::default().fg(Color::Gray),
        )));
    }
    f.render_widget(List::new(items).block(panel(" WIRING ")), right);
}

fn nmea_color(line: &str) -> Color {
    match line.get(3..6) {
        Some("GGA" | "GNS") => Color::Green,
        Some("RMC") => Color::Yellow,
        Some("GSV") => Color::Cyan,
        Some("GSA") => Color::Magenta,
        Some("VTG" | "GLL") => Color::LightBlue,
        Some("TXT") => Color::White,
        _ => Color::Gray,
    }
}

pub fn raw(v: &View, height: u16, paused: bool) -> List<'_> {
    let rows = height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = v.raw[v.raw.len().saturating_sub(rows)..]
        .iter()
        .map(|l| ListItem::new(l.as_ref()).style(Style::default().fg(nmea_color(l))))
        .collect();
    let title = if paused {
        " RAW NMEA — PAUSED (p to resume) "
    } else {
        " RAW NMEA "
    };
    List::new(items).block(Block::default().title(title).borders(Borders::ALL))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges() {
        assert_eq!(fix_badge(None, None).0, "NO DATA");
        let mut f = Fix {
            valid: true,
            mode: FixMode::Fix3D,
            quality: FixQuality::Gps,
            ..Fix::default()
        };
        assert_eq!(fix_badge(Some(&f), Some(0.2)).0, "3D FIX");
        assert_eq!(fix_badge(Some(&f), Some(10.0)).0, "STALE");
        f.quality = FixQuality::Rtk;
        assert_eq!(fix_badge(Some(&f), Some(0.2)).0, "RTK FIXED");
        f.valid = false;
        assert_eq!(fix_badge(Some(&f), Some(0.2)).0, "NO FIX");
    }

    #[test]
    fn dms_format() {
        assert_eq!(dms(56.5, 'N', 'S'), "56°30'00.000\"N");
        assert_eq!(dms(-24.25, 'E', 'W'), "24°15'00.000\"W");
    }

    #[test]
    fn nmea_colors_by_formatter() {
        assert_eq!(nmea_color("$GNGGA,1"), Color::Green);
        assert_eq!(nmea_color("$GPGSV,1"), Color::Cyan);
        assert_eq!(nmea_color("$PUBX,00"), Color::Gray);
    }
}
