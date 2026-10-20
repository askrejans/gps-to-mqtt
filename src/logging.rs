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
    capacity: usize,
}

impl TuiWriter {
    pub fn new(buffer: Arc<Mutex<VecDeque<String>>>, capacity: usize) -> Self {
        Self {
            buffer,
            capacity: capacity.max(1),
        }
    }
}

impl Write for TuiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let trimmed = s.trim_end_matches('\n');
            if !trimmed.is_empty() {
                let mut guard = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
                if guard.len() >= self.capacity {
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
pub fn init_service(config: &AppConfig) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    if config.log_json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        // No colour codes when stdout is journald, a pipe or a file.
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
            .init();
    }
}

/// Initialize logging for interactive TUI mode.
///
/// All log output is captured by `writer` and fed into the on-screen log panel;
/// nothing is written to stdout so the terminal display stays clean.
pub fn init_tui(config: &AppConfig, writer: TuiWriter) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_writer_respects_capacity() {
        let buf = Arc::new(Mutex::new(VecDeque::new()));
        let mut w = TuiWriter::new(buf.clone(), 3);
        for i in 0..5 {
            // tracing-subscriber formats a whole record, then writes it once.
            w.write_all(format!("line {i}\n").as_bytes()).unwrap();
        }
        let lines: Vec<String> = buf.lock().unwrap().iter().cloned().collect();
        assert_eq!(lines, ["line 2", "line 3", "line 4"]);
    }
}
