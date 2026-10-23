//! Interactive terminal dashboard (started automatically on a TTY).
//!
//! ```text
//! ┌ gps-to-mqtt 0.7.0 │ ● /dev/cu.usbmodem1101 @ 9600 │ 3D FIX · 0.1 s │ MQTT ● │ 21 sent/s · 1 fix/s ┐
//! │ 1 Overview  2 Satellites  3 Stats  4 Logs  5 Raw NMEA                    q quit · p pause │
//! ├──────────────── POSITION ─────────┬──────── FIX ─────────┬──────── DYNAMICS (g-g) ───────┤
//! │ …                                 │ …                    │            ·                  │
//! ├────────────────────────────────── LOG ─────────────────────────────────────────────────────┤
//! └───────────────────────────────────────────────────────────────────────────────────────────┘
//! ```

mod widgets;

use crate::shared::{Counters, Shared};
use anyhow::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
};
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const TABS: [&str; 5] = [
    "1 Overview",
    "2 Satellites",
    "3 Stats",
    "4 Logs",
    "5 Raw NMEA",
];

/// Everything one frame needs, copied out of the shared state so no lock is
/// held while drawing.
pub struct View {
    pub fix: Option<Arc<crate::models::Fix>>,
    pub fix_age: Option<f64>,
    pub sky: Arc<crate::models::Sky>,
    pub raw: Vec<Arc<str>>,
    pub messages: Vec<String>,
    pub source: crate::shared::LinkStatus,
    pub source_detail: String,
    pub mqtt: crate::shared::LinkStatus,
    pub sentence_rate: f64,
    pub fix_rate: f64,
    pub logs: Vec<String>,
    pub counters: [(&'static str, u64); 9],
    pub info: crate::shared::Info,
    pub uptime: Duration,
    pub replay: bool,
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
}

pub async fn run(
    shared: Arc<Shared>,
    logs: Arc<Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
    refresh_ms: u64,
    replay: bool,
) -> Result<()> {
    // Always give the terminal back, even if a render panics.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let result = event_loop(
        &mut terminal,
        shared,
        logs,
        cancel,
        refresh_ms.clamp(16, 2000),
        replay,
    )
    .await;
    restore_terminal();
    result
}

#[derive(Default)]
struct UiState {
    tab: usize,
    paused_raw: Option<Vec<Arc<str>>>,
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    shared: Arc<Shared>,
    logs: Arc<Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
    refresh_ms: u64,
    replay: bool,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(refresh_ms));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ui = UiState::default();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tick.tick() => {
                let view = snapshot(&shared, &logs, &ui, replay);
                terminal.draw(|f| render(f, &view, &ui))?;
            }
            ev = events.next() => {
                let Some(Ok(Event::Key(k))) = ev else { continue };
                if k.kind != KeyEventKind::Press { continue; }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => { cancel.cancel(); return Ok(()); }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    KeyCode::Char(c @ '1'..='5') => ui.tab = (c as u8 - b'1') as usize,
                    KeyCode::Right | KeyCode::Tab => ui.tab = (ui.tab + 1) % TABS.len(),
                    KeyCode::Left | KeyCode::BackTab => ui.tab = (ui.tab + TABS.len() - 1) % TABS.len(),
                    KeyCode::Char('p') => {
                        ui.paused_raw = match ui.paused_raw.take() {
                            Some(_) => None,
                            None => Some(shared.read().raw.iter().cloned().collect()),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}

fn snapshot(shared: &Shared, logs: &Mutex<VecDeque<String>>, ui: &UiState, replay: bool) -> View {
    let c = &shared.counters;
    let logs: Vec<String> = logs
        .lock()
        .map(|l| l.iter().cloned().collect())
        .unwrap_or_default();
    let s = shared.read();
    View {
        fix: s.fix.clone(),
        fix_age: s.fix_age_s(Instant::now()),
        sky: s.sky.clone(),
        raw: ui
            .paused_raw
            .clone()
            .unwrap_or_else(|| s.raw.iter().cloned().collect()),
        messages: s.messages.iter().cloned().collect(),
        source: s.source,
        source_detail: s.source_detail.clone(),
        mqtt: s.mqtt,
        sentence_rate: s.sentence_rate,
        fix_rate: s.fix_rate,
        logs,
        counters: [
            ("Bytes received", Counters::get(&c.bytes)),
            ("NMEA sentences", Counters::get(&c.sentences)),
            ("Checksum errors", Counters::get(&c.checksum_errors)),
            ("Malformed lines", Counters::get(&c.malformed)),
            ("Fixes emitted", Counters::get(&c.fixes)),
            ("MQTT published", Counters::get(&c.mqtt_published)),
            ("MQTT dropped", Counters::get(&c.mqtt_dropped)),
            ("Archive errors", Counters::get(&c.archive_errors)),
            ("TCP clients", Counters::get(&c.clients)),
        ],
        info: shared.info.clone(),
        uptime: shared.started.elapsed(),
        replay,
    }
}

fn render(f: &mut Frame, v: &View, ui: &UiState) {
    let [header, tabs, main, log] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(7),
    ])
    .areas(f.area());

    f.render_widget(widgets::header(v), header);
    render_tabs(f, tabs, ui);
    match ui.tab {
        0 => widgets::overview(f, main, v),
        1 => widgets::satellites(f, main, v),
        2 => widgets::stats(f, main, v),
        3 => f.render_widget(log_list(&v.logs, main.height, " APPLICATION LOG "), main),
        _ => f.render_widget(widgets::raw(v, main.height, ui.paused_raw.is_some()), main),
    }
    f.render_widget(log_list(&v.logs, log.height, " LOG "), log);
}

fn render_tabs(f: &mut Frame, area: Rect, ui: &UiState) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(30)]).areas(area);
    f.render_widget(
        Tabs::new(TABS.to_vec())
            .select(ui.tab)
            .style(Style::default().fg(Color::Gray))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        left,
    );
    let hint = Line::from(vec![
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit · "),
        Span::styled("p", Style::default().fg(Color::Yellow)),
        Span::raw(if ui.paused_raw.is_some() {
            " resume raw"
        } else {
            " pause raw"
        }),
        Span::raw(" · ←/→"),
    ])
    .right_aligned();
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        right,
    );
}

pub fn log_style(line: &str) -> Style {
    if line.contains("ERROR") {
        Style::default().fg(Color::Red)
    } else if line.contains("WARN") {
        Style::default().fg(Color::Yellow)
    } else if line.contains("INFO") {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn log_list<'a>(logs: &'a [String], height: u16, title: &'a str) -> List<'a> {
    let rows = height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = logs[logs.len().saturating_sub(rows)..]
        .iter()
        .map(|l| ListItem::new(l.as_str()).style(log_style(l)))
        .collect();
    List::new(items).block(Block::default().title(title).borders(Borders::ALL))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Constellation, Fix, FixMode, FixQuality, Satellite, Sky, Telemetry};
    use crate::shared::{Info, LinkStatus};
    use ratatui::backend::TestBackend;

    pub fn sample_view() -> View {
        let sat = |c, prn, el, az, snr, used| Satellite {
            constellation: c,
            prn,
            elevation_deg: Some(el),
            azimuth_deg: Some(az),
            snr_dbhz: Some(snr),
            used,
        };
        View {
            fix: Some(Arc::new(Fix {
                valid: true,
                mode: FixMode::Fix3D,
                quality: FixQuality::Gps,
                latitude: Some(56.9637),
                mgrs: crate::mgrs::mgrs(56.9637, 24.2289, 10),
                longitude: Some(24.2289),
                altitude_m: Some(12.3),
                speed_mps: Some(41.7),
                course_deg: Some(87.5),
                satellites_used: Some(12),
                hdop: Some(0.9),
                vdop: Some(1.4),
                pdop: Some(1.7),
                accuracy_m: Some(4.5),
                accuracy_source: Some(crate::models::AccuracySource::HdopEstimate),
                telemetry: Some(Telemetry {
                    longitudinal_g: Some(-0.62),
                    lateral_g: Some(0.85),
                    combined_g: Some(1.05),
                    yaw_rate_dps: Some(12.0),
                    distance_m: 4321.0,
                    max_speed_mps: 55.0,
                    braking: true,
                    ..Telemetry::default()
                }),
                ..Fix::default()
            })),
            fix_age: Some(0.1),
            sky: Arc::new(Sky {
                satellites: vec![
                    sat(Constellation::Gps, 7, 67, 182, 37, true),
                    sat(Constellation::Glonass, 66, 68, 89, 25, true),
                    sat(Constellation::Galileo, 3, 20, 300, 18, false),
                ],
                in_view: 3,
                used: 2,
                hdop: Some(0.9),
                vdop: None,
                pdop: None,
            }),
            raw: vec![Arc::from(
                "$GNGGA,084047.00,5657.81857,N,02413.73641,E,1,09,1.79,76.2,M,22.1,M,,*78",
            )],
            messages: vec!["ANTSTATUS=OK".into()],
            source: LinkStatus::Connected,
            source_detail: "/dev/ttyACM0 @ 115200 baud".into(),
            mqtt: LinkStatus::Connected,
            sentence_rate: 21.0,
            fix_rate: 10.0,
            logs: vec!["2026-10-20T08:00:00Z  INFO receiver: ANTSTATUS=OK".into()],
            counters: [("NMEA sentences", 559); 9],
            info: Info {
                version: "0.7.0",
                source: "auto @ auto baud".into(),
                receiver: "ublox @ 10 hz".into(),
                mqtt: Some("localhost:1883".into()),
                outputs: vec!["mqtt://localhost:1883 [legacy,json,canonical]".into()],
            },
            uptime: Duration::from_secs(3723),
            replay: false,
        }
    }

    fn draw(tab: usize) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        let ui = UiState {
            tab,
            paused_raw: None,
        };
        terminal.draw(|f| render(f, &sample_view(), &ui)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_tab_renders_key_information() {
        let overview = draw(0);
        for needle in [
            "3D FIX",
            "/dev/ttyACM0 @ 115200 baud",
            "150.1 km/h",
            "56°57'49.320\"N",
            "BRAKE",
            "35V LD 31524 16761",
            "±4.5 m (hdop-estimate)",
            "21 sent/s · 10 fix/s",
        ] {
            assert!(
                overview.contains(needle),
                "overview missing {needle:?}\n{overview}"
            );
        }
        let sats = draw(1);
        for needle in ["3 in view · 2 used", "G07", "R66", "E03", "SKY VIEW"] {
            assert!(
                sats.contains(needle),
                "satellites missing {needle:?}\n{sats}"
            );
        }
        let stats = draw(2);
        for needle in [
            "NMEA sentences",
            "1h 02m 03s",
            "mqtt://localhost:1883",
            "ANTSTATUS=OK",
        ] {
            assert!(stats.contains(needle), "stats missing {needle:?}\n{stats}");
        }
        assert!(draw(4).contains("$GNGGA,084047.00"));
        if std::env::var_os("SHOW_TUI").is_some() {
            println!("{overview}\n\n{sats}");
        }
    }
}
