use crate::config::AppConfig;
use crate::models::AppMode;
use anyhow::Result;
use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize logging based on the application mode
pub fn init_logging(config: &AppConfig) -> Result<()> {
    let log_level = parse_log_level(&config.log_level);

    match config.mode {
        AppMode::Tui => init_tui_logging(log_level),
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

/// Initialize logging for TUI mode - compact format that can be captured
fn init_tui_logging(log_level: Level) -> Result<()> {
    let filter = EnvFilter::from_default_env()
        .add_directive(log_level.into())
        .add_directive("gps_to_mqtt=trace".parse()?);

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
