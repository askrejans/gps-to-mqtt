//! Logging infrastructure.
//!
//! Two modes:
//! - **TUI mode** — log lines captured into a shared ring-buffer via `TuiWriter`;
//!   the bottom log panel of the TUI renders the buffer in real time.
//! - **Service mode** — human-readable (pretty) or JSON structured output to
//!   stdout (journald / log aggregation friendly).

use crate::config::AppConfig;
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// TUI log writer
// ---------------------------------------------------------------------------

/// An `io::Write` implementation that appends formatted log lines to a shared
/// ring-buffer, which the TUI renders in the bottom log panel.
#[derive(Clone)]
pub struct TuiWriter {
    buffer: Arc<Mutex<VecDeque<String>>>,
}

impl TuiWriter {
    pub fn new(buffer: Arc<Mutex<VecDeque<String>>>) -> Self {
        Self { buffer }
    }
}

impl Write for TuiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let trimmed = s.trim_end_matches('\n');
            if !trimmed.is_empty() {
                let mut guard = self.buffer.lock().unwrap();
                if guard.len() >= 500 {
                    guard.pop_front();
                }
                guard.push_back(trimmed.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TuiWriter {
    type Writer = TuiWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Logging initializers
// ---------------------------------------------------------------------------

/// Initialize logging for service / daemon mode.
///
/// Writes to stdout — pretty text or JSON depending on `config.log_json`.
pub fn init_logging_service(config: &AppConfig) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    if config.log_json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    }
}

/// Initialize logging for interactive TUI mode.
///
/// All log output is captured by `writer` and fed into the on-screen log panel;
/// nothing is written to stdout so the terminal display stays clean.
pub fn init_logging_tui(config: &AppConfig, writer: TuiWriter) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
}
