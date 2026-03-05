//! Terminal User Interface
//!
//! Renders a live four-tab dashboard when the application is run interactively
//! (TTY detected).  In service / daemon mode the TUI is skipped and structured
//! logs are written to stdout.
//!
//! # Layout
//! ```text
//! ┌─────────────────── GPS-to-MQTT vX.Y ─ press q to quit ──────────────────┐
//! │ Overview(1) │ Satellites(2) │ Logs(3) │ Raw GPS(4)                       │
//! ├───────────────────────────────────────────────────────────────────────────┤
//! │ CONNECTIONS         │ GPS DATA (tab content)                              │
//! │ GPS: ● ONLINE       │ ── POSITION ──                                      │
//! │  /dev/ttyUSB0       │  Latitude  …                                        │
//! │ MQTT: ● ONLINE      │                                                     │
//! │  localhost:1883     │                                                     │
//! ├───────────────────────────────────────────────────────────────────────────┤
//! │ LOG (most recent)                                                          │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```

mod widgets;

use crate::models::{AppState, GpsData, MqttStatus};
use anyhow::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::RwLock;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

pub use widgets::*;

// ---------------------------------------------------------------------------
// State snapshot for render pass
// ---------------------------------------------------------------------------

struct StateSnapshot {
    serial_connected: bool,
    mqtt_connected: bool,
    mqtt_enabled: bool,
    connection_address: String,
    mqtt_address: String,
    gps_data: GpsData,
    messages_published: u64, // snapshot of AtomicU64
    logs: Vec<String>,
    selected_tab: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the interactive TUI until `q` / Ctrl+C is pressed or `cancel` fires.
pub async fn run_tui(
    state: Arc<RwLock<AppState>>,
    log_buffer: Arc<Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = tui_loop(&mut terminal, state, log_buffer, cancel).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ---------------------------------------------------------------------------
// Internal loop
// ---------------------------------------------------------------------------

async fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
    log_buffer: Arc<Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut event_stream = EventStream::new();
    let mut render_tick = interval(Duration::from_millis(100));
    // Satellites and sky-chart update at 1 Hz to stay human-readable at 10 Hz GPS rate
    let mut sat_tick = interval(Duration::from_secs(1));
    let mut selected_tab: usize = 0;

    // Cached satellite snapshot — only refreshed on sat_tick
    let mut sat_cache: std::collections::HashMap<u32, crate::models::SatelliteInfo> =
        std::collections::HashMap::new();
    let mut sat_count_cache: Option<u32> = None;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,

            // Refresh satellite cache at 1 Hz; drop entries not seen in the last 10 s
            _ = sat_tick.tick() => {
                let s = state.read().await;
                let horizon = std::time::Duration::from_secs(10);
                sat_cache = s.gps_data.satellites
                    .iter()
                    .filter(|(_, sat)| sat.last_seen.elapsed() < horizon)
                    .map(|(k, v)| (*k, v.clone()))
                    .collect();
                sat_count_cache = if sat_cache.is_empty() {
                    None
                } else {
                    Some(sat_cache.len() as u32)
                };
            }

            _ = render_tick.tick() => {
                let s = state.read().await;
                let logs: Vec<String> = log_buffer.lock().unwrap().iter().cloned().collect();
                // Build GPS data with live navigation but throttled satellites
                let mut gps_data = s.gps_data.clone();
                gps_data.satellites = sat_cache.clone();
                gps_data.satellites_in_view = sat_count_cache;
                let snap = StateSnapshot {
                    serial_connected: s.serial_connected,
                    mqtt_connected: s.mqtt_status == MqttStatus::Connected,
                    mqtt_enabled: s.mqtt_enabled,
                    connection_address: s.connection_address.clone(),
                    mqtt_address: s.mqtt_address.clone(),
                    gps_data,
                    messages_published: s.messages_published.load(std::sync::atomic::Ordering::Relaxed),
                    logs,
                    selected_tab,
                };
                drop(s);
                terminal.draw(|f| render(f, &snap))?;
            }

            Some(Ok(event)) = event_stream.next() => {
                match event {
                    Event::Key(k) if k.code == KeyCode::Char('q')
                        || (k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)) =>
                    {
                        cancel.cancel();
                        break;
                    }
                    Event::Key(k) => match k.code {
                        KeyCode::Char('1') => selected_tab = 0,
                        KeyCode::Char('2') => selected_tab = 1,
                        KeyCode::Char('3') => selected_tab = 2,
                        KeyCode::Char('4') => selected_tab = 3,
                        KeyCode::Left if selected_tab > 0 => selected_tab -= 1,
                        KeyCode::Right if selected_tab < 3 => selected_tab += 1,
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(f: &mut Frame, snap: &StateSnapshot) {
    let area = f.area();

    // Vertical: header(3) | tabs(1) | main(min) | log(8)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(8),
        ])
        .split(area);

    render_header(f, vertical[0]);
    render_tabs(f, vertical[1], snap.selected_tab);
    render_main(f, vertical[2], snap);
    render_log(f, vertical[3], snap);
}

fn render_header(f: &mut Frame, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            concat!(" GPS-to-MQTT v", env!("CARGO_PKG_VERSION"), " "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ press "),
        Span::styled(
            "q",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, area);
}

fn render_tabs(f: &mut Frame, area: Rect, selected: usize) {
    let tabs = Tabs::new(vec![
        "Overview (1)",
        "Satellites (2)",
        "App Logs (3)",
        "Raw GPS (4)",
    ])
    .select(selected)
    .style(Style::default().fg(Color::White))
    .highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(tabs, area);
}

fn render_main(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    match snap.selected_tab {
        0 => render_overview(f, area, snap),
        1 => render_satellites(f, area, snap),
        2 => render_full_logs(f, area, snap),
        3 => render_raw_gps(f, area, snap),
        _ => {}
    }
}

// ── Tab 1: Overview ──────────────────────────────────────────────────────────

fn render_overview(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    // Left: connections panel (28 cols fixed); Right: GPS data
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(area);

    let conn_widget = render_connections_widget(&AppState {
        serial_connected: snap.serial_connected,
        mqtt_enabled: snap.mqtt_enabled,
        mqtt_status: if snap.mqtt_connected {
            MqttStatus::Connected
        } else {
            MqttStatus::Disconnected
        },
        connection_address: snap.connection_address.clone(),
        mqtt_address: snap.mqtt_address.clone(),
        messages_published: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
            snap.messages_published,
        )),
        gps_data: Default::default(),
    });
    f.render_widget(conn_widget, horiz[0]);

    let data_widget = render_gps_data_widget(&snap.gps_data);
    f.render_widget(data_widget, horiz[1]);
}

// ── Tab 2: Satellites ────────────────────────────────────────────────────────

fn render_satellites(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let sat_list = create_satellite_list_widget(&snap.gps_data);
    f.render_widget(sat_list, horiz[0]);

    let sky = create_satellite_sky_chart(&snap.gps_data);
    f.render_widget(sky, horiz[1]);
}

// ── Tab 3: Full log ──────────────────────────────────────────────────────────

fn render_full_logs(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let max_lines = area.height.saturating_sub(2) as usize;
    let start = snap.logs.len().saturating_sub(max_lines);
    let items: Vec<ListItem> = snap.logs[start..]
        .iter()
        .map(|line| {
            let style = log_line_style(line);
            ListItem::new(line.as_str()).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title(" APP LOGS ").borders(Borders::ALL));
    f.render_widget(list, area);
}

// ── Tab 4: Raw NMEA ──────────────────────────────────────────────────────────

fn render_raw_gps(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let max_lines = area.height.saturating_sub(2) as usize;
    let buf = &snap.gps_data.raw_nmea_buffer;
    let start = buf.len().saturating_sub(max_lines);
    let items: Vec<ListItem> = buf[start..]
        .iter()
        .map(|sentence| {
            let style = if sentence.starts_with("$GPGGA") || sentence.starts_with("$GNGGA") {
                Style::default().fg(Color::Green)
            } else if sentence.starts_with("$GPGSV")
                || sentence.starts_with("$GLGSV")
                || sentence.starts_with("$GAGSV")
            {
                Style::default().fg(Color::Cyan)
            } else if sentence.starts_with("$GPRMC") || sentence.starts_with("$GNRMC") {
                Style::default().fg(Color::Yellow)
            } else if sentence.starts_with("$GPGSA") || sentence.starts_with("$GNGSA") {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(sentence.as_str()).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title(" RAW NMEA ").borders(Borders::ALL));
    f.render_widget(list, area);
}

// ── Shared log panel (bottom, always visible) ────────────────────────────────

fn render_log(f: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let max_lines = area.height.saturating_sub(2) as usize;
    let start = snap.logs.len().saturating_sub(max_lines);
    let items: Vec<ListItem> = snap.logs[start..]
        .iter()
        .map(|line| ListItem::new(line.as_str()).style(log_line_style(line)))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" LOG (most recent) ")
            .borders(Borders::ALL),
    );
    f.render_widget(list, area);
}

fn log_line_style(line: &str) -> Style {
    if line.contains("ERROR") || line.contains("Error") {
        Style::default().fg(Color::Red)
    } else if line.contains("WARN") || line.contains("Warning") {
        Style::default().fg(Color::Yellow)
    } else if line.contains("INFO") || line.contains("connected") || line.contains("Connected") {
        Style::default().fg(Color::Gray)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}
