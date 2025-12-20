use crate::config::AppConfig;
use crate::models::AppMode;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Initialize logging based on the application mode
pub fn init_logging(config: &AppConfig) -> Result<()> {
    init_logging_with_buffer(config, None)
}

/// Initialize logging with optional buffer for TUI mode
pub fn init_logging_with_buffer(
    config: &AppConfig,
    log_buffer: Option<Arc<RwLock<Vec<String>>>>,
) -> Result<()> {
    let log_level = parse_log_level(&config.log_level);

    match config.mode {
        AppMode::Tui => init_tui_logging(log_level, log_buffer),
        AppMode::Cli => init_cli_logging(log_level),
        AppMode::Service => init_service_logging(config, log_level),
    }
}

/// Parse log level string to tracing Level
fn parse_log_level(level: &str) -> Level {
    match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Initialize logging for TUI mode - capture logs to buffer without printing to stdout
fn init_tui_logging(
    log_level: Level,
    log_buffer: Option<Arc<RwLock<Vec<String>>>>,
) -> Result<()> {
    let filter = EnvFilter::from_default_env()
        .add_directive(log_level.into())
        .add_directive("rumqttc=warn".parse()?)
        .add_directive("tokio=warn".parse()?)
        .add_directive("gps_to_mqtt=info".parse()?);

    if let Some(buffer) = log_buffer {
        // Use custom layer that captures to buffer
        tracing_subscriber::registry()
            .with(filter)
            .with(LogBufferLayer::new(buffer))
            .init();
    } else {
        // Fallback to null output (shouldn't happen)
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(std::io::sink)
                    .compact()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_file(false)
                    .with_line_number(false),
            )
            .init();
    }

    Ok(())
}

/// Initialize logging for CLI mode - human-readable console output
fn init_cli_logging(log_level: Level) -> Result<()> {
    let filter = EnvFilter::from_default_env()
        .add_directive(log_level.into())
        .add_directive("rumqttc=warn".parse()?)
        .add_directive("tokio=warn".parse()?)
        .add_directive("gps_to_mqtt=info".parse()?);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .compact()
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
        .init();

    Ok(())
}

/// Initialize logging for service mode - structured logging to file/journald
fn init_service_logging(config: &AppConfig, log_level: Level) -> Result<()> {
    let filter = EnvFilter::from_default_env()
        .add_directive(log_level.into())
        .add_directive("gps_to_mqtt=trace".parse()?);

    if let Some(log_path) = &config.log_file_path {
        // Log to file with rotation
        let file_appender = tracing_appender::rolling::daily(
            std::path::Path::new(log_path)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("/var/log")),
            std::path::Path::new(log_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("gps-to-mqtt.log"),
        );

        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .json()
                    .with_writer(file_appender)
                    .with_current_span(true)
                    .with_span_list(false),
            )
            .init();
    } else {
        // Log to stdout in JSON format (for journald)
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(false),
            )
            .init();
    }

    Ok(())
}

/// Custom tracing layer that captures log messages to a buffer
struct LogBufferLayer {
    buffer: Arc<RwLock<Vec<String>>>,
}

impl LogBufferLayer {
    fn new(buffer: Arc<RwLock<Vec<String>>>) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let mut message = String::new();
        
        // Format: LEVEL message
        let level = metadata.level();
        message.push_str(&format!("{:5} ", level));
        
        // Visit fields to extract message
        let mut visitor = MessageVisitor { message: String::new() };
        event.record(&mut visitor);
        message.push_str(&visitor.message);
        
        // Store in buffer (limit to last 1000 lines)
        if let Ok(mut log_buffer) = self.buffer.try_write() {
            log_buffer.push(message);
            if log_buffer.len() > 1000 {
                log_buffer.drain(0..100);
            }
        }
    }
}

struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
            // Remove surrounding quotes from debug output
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len()-1].to_string();
            }
        } else {
            if !self.message.is_empty() {
                self.message.push_str(", ");
            }
            self.message.push_str(&format!("{}: {:?}", field.name(), value));
        }
    }
}
