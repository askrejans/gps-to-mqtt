mod widgets;

use crate::config::AppConfig;
use crate::models::{AppState, MqttStatus};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

pub use widgets::*;

/// TUI Application state
pub struct TuiApp {
    state: Arc<RwLock<AppState>>,
    log_buffer: Arc<RwLock<Vec<String>>>,
    selected_tab: usize,
    should_quit: bool,
}

impl TuiApp {
    pub fn new(state: Arc<RwLock<AppState>>, log_buffer: Arc<RwLock<Vec<String>>>) -> Self {
        Self {
            state,
            log_buffer,
            selected_tab: 0,
            should_quit: false,
        }
    }

    /// Run the TUI application
    pub async fn run(mut self, config: AppConfig) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let refresh_rate = Duration::from_millis(config.tui_refresh_rate_ms);

        // Main loop
        loop {
            let state = self.state.read().await.clone();
            let logs = self.log_buffer.read().await.clone();

            terminal.draw(|f| self.draw(f, &state, &logs))?;

            // Handle events with timeout
            if event::poll(refresh_rate)? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            self.should_quit = true;
                        }
                        KeyCode::Char('1') => self.selected_tab = 0,
                        KeyCode::Char('2') => self.selected_tab = 1,
                        KeyCode::Char('3') => self.selected_tab = 2,
                        KeyCode::Left => {
                            if self.selected_tab > 0 {
                                self.selected_tab -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if self.selected_tab < 2 {
                                self.selected_tab += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        info!("TUI application exited");
        Ok(())
    }

    /// Draw the UI
    fn draw(&self, f: &mut Frame, state: &AppState, logs: &[String]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header with tabs
                Constraint::Min(0),     // Main content
                Constraint::Length(3),  // Status bar
            ])
            .split(f.area());

        // Render header with tabs
        self.render_header(f, chunks[0]);

        // Render content based on selected tab
        match self.selected_tab {
            0 => self.render_overview(f, chunks[1], state),
            1 => self.render_satellites(f, chunks[1], state),
            2 => self.render_logs(f, chunks[1], logs),
            _ => {}
        }

        // Render status bar
        self.render_status_bar(f, chunks[2], state);
    }

    /// Render header with tabs
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let titles = vec!["Overview (1)", "Satellites (2)", "Logs (3)"];
        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title("GPS to MQTT Monitor"))
            .select(self.selected_tab)
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(tabs, area);
    }

    /// Render overview tab
    fn render_overview(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10), // Position info
                Constraint::Length(8),  // Fix info
                Constraint::Min(0),     // Messages
            ])
            .split(area);

        // Position information
        let position_widget = create_position_widget(&state.gps_data);
        f.render_widget(position_widget, chunks[0]);

        // Fix information
        let fix_widget = create_fix_widget(&state.gps_data);
        f.render_widget(fix_widget, chunks[1]);

        // Recent messages
        let messages_widget = create_messages_widget(&state.gps_data);
        f.render_widget(messages_widget, chunks[2]);
    }

    /// Render satellites tab
    fn render_satellites(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Satellite list
        let sat_list = create_satellite_list_widget(&state.gps_data);
        f.render_widget(sat_list, chunks[0]);

        // Satellite sky chart
        let sky_chart = create_satellite_sky_chart(&state.gps_data);
        f.render_widget(sky_chart, chunks[1]);
    }

    /// Render logs tab
    fn render_logs(&self, f: &mut Frame, area: Rect, logs: &[String]) {
        let log_items: Vec<ListItem> = logs
            .iter()
            .rev() // Show newest first
            .take(area.height as usize - 2) // Account for borders
            .map(|log| {
                let style = if log.contains("ERROR") {
                    Style::default().fg(Color::Red)
                } else if log.contains("WARN") {
                    Style::default().fg(Color::Yellow)
                } else if log.contains("INFO") {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(log.as_str()).style(style)
            })
            .collect();

        let logs_widget = List::new(log_items)
            .block(Block::default().borders(Borders::ALL).title("Logs"));

        f.render_widget(logs_widget, area);
    }

    /// Render status bar
    fn render_status_bar(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let mqtt_status_str = match state.mqtt_status {
            MqttStatus::Connected => "MQTT: Connected",
            MqttStatus::Connecting => "MQTT: Connecting...",
            MqttStatus::Disconnected => "MQTT: Disconnected",
            MqttStatus::Error => "MQTT: Error",
        };

        let mqtt_color = match state.mqtt_status {
            MqttStatus::Connected => Color::Green,
            MqttStatus::Connecting => Color::Yellow,
            MqttStatus::Disconnected => Color::Gray,
            MqttStatus::Error => Color::Red,
        };

        let serial_status_str = if state.serial_connected {
            "Serial: Connected"
        } else {
            "Serial: Disconnected"
        };

        let serial_color = if state.serial_connected {
            Color::Green
        } else {
            Color::Red
        };

        let status_line = Line::from(vec![
            Span::styled(mqtt_status_str, Style::default().fg(mqtt_color)),
            Span::raw(" | "),
            Span::styled(serial_status_str, Style::default().fg(serial_color)),
            Span::raw(" | "),
            Span::styled("Press 'q' or ESC to quit", Style::default().fg(Color::Gray)),
        ]);

        let status_bar = Paragraph::new(status_line)
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(status_bar, area);
    }
}
