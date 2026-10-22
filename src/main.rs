#![cfg_attr(
    test,
    allow(clippy::field_reassign_with_default, clippy::type_complexity)
)]

mod archive;
mod clock;
mod config;
mod engine;
mod health;
mod hold;
mod hub;
mod logging;
mod mgrs;
mod models;
mod nmea;
mod output;
mod position;
mod receiver;
mod service;
mod shared;
mod source;
mod telemetry;
mod ui;

use anyhow::Result;
use clap::Parser;
use config::{AppConfig, ConnectionType, load_configuration};
use shared::{Info, Shared};
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Universal GNSS (NMEA-0183) bridge to MQTT, HTTP, gpsd and NMEA network outputs.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Path to a TOML config file (default: standard locations, then ./settings.toml)
    #[arg(short, long, value_name = "FILE")]
    config: Option<String>,

    /// Validate the configuration, print the effective settings and exit
    #[arg(long)]
    check_config: bool,

    /// List serial ports ranked by GNSS likelihood and exit
    #[arg(long)]
    list_ports: bool,

    /// With --list-ports: open each port and probe for NMEA at common baud rates
    #[arg(long, requires = "list_ports")]
    probe: bool,

    /// Replay an NMEA log instead of reading a device (implies connection_type = file)
    #[arg(long, value_name = "FILE")]
    replay: Option<String>,

    /// Never start the TUI, even on a terminal
    #[arg(long)]
    headless: bool,

    /// Query the local HTTP /health endpoint and exit 0 (healthy) or 1 (container probes)
    #[arg(long)]
    healthcheck: bool,
}

/// `--healthcheck`: minimal HTTP/1.0 GET so container images need no curl.
fn healthcheck(config: &AppConfig) -> Result<bool> {
    use std::io::{Read, Write};
    anyhow::ensure!(
        config.http_enabled,
        "http_enabled is false; nothing to probe"
    );
    let host = match config.http_bind.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        h => h,
    };
    let addr: std::net::SocketAddr = format!("{host}:{}", config.http_port).parse()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response)?;
    let ok = response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200");
    println!("{}", response.lines().last().unwrap_or_default());
    Ok(ok)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.list_ports {
        source::serial::list_ports(cli.probe);
        return Ok(());
    }

    let mut config = load_configuration(cli.config.as_deref())?;
    if let Some(path) = cli.replay {
        config.connection_type = ConnectionType::File;
        config.replay_file = Some(path);
        config.validate()?;
    }

    if cli.healthcheck {
        let healthy = healthcheck(&config).unwrap_or_else(|e| {
            eprintln!("healthcheck failed: {e:#}");
            false
        });
        std::process::exit(if healthy { 0 } else { 1 });
    }

    if cli.check_config {
        println!("{}", serde_json::to_string_pretty(&config.redacted())?);
        println!(
            "\nConfiguration OK ({})",
            config
                .config_path
                .as_deref()
                .unwrap_or("defaults + environment")
        );
        return Ok(());
    }

    let tui = !cli.headless && std::io::stdout().is_terminal();
    let log_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(
        config.max_log_buffer_size,
    )));
    if tui {
        logging::init_tui(
            &config,
            logging::TuiWriter::new(log_buffer.clone(), config.max_log_buffer_size),
        );
    } else {
        logging::init_service(&config);
    }
    info!("gps-to-mqtt v{} starting", env!("CARGO_PKG_VERSION"));
    match &config.config_path {
        Some(p) => info!("Configuration: {p}"),
        None => warn!("No configuration file found; using defaults and environment"),
    }

    let outputs = output::describe(&config);
    let shared = Shared::new(
        Info {
            version: env!("CARGO_PKG_VERSION"),
            source: config.connection_display(),
            receiver: match config.effective_update_rate() {
                Some(hz) => format!("{:?} @ {hz} Hz", config.effective_receiver()).to_lowercase(),
                None => format!("{:?}", config.effective_receiver()).to_lowercase(),
            },
            mqtt: config.mqtt_enabled.then(|| config.mqtt_address()),
            outputs: outputs.clone(),
        },
        config.mqtt_enabled,
    );
    info!(
        "Source: {} · receiver: {}",
        shared.info.source, shared.info.receiver
    );
    if outputs.is_empty() {
        warn!("No outputs enabled — data is only shown locally");
    }
    for o in &outputs {
        info!("Output: {o}");
    }

    let cancel = CancellationToken::new();
    let mut tasks = JoinSet::new();

    let archive = config
        .telemetry_log
        .clone()
        .map(|path| archive::spawn(path, shared.clone()));

    let (source_tx, source_rx) = mpsc::channel(256);
    {
        let (s, k) = (shared.clone(), cancel.clone());
        let opts = hub::HubOptions {
            require_checksum: config.nmea_require_checksum,
            telemetry_window: config
                .telemetry_enabled
                .then_some(config.telemetry_smoothing_window),
            satellite_timeout: Duration::from_secs(config.satellite_timeout_s),
            static_hold: config.static_hold.then(|| hold::HoldConfig {
                speed_mps: config.static_hold_speed_kph / 3.6,
                radius_m: config.static_hold_radius_m,
            }),
            archive,
        };
        tasks.spawn(hub::run(source_rx, s, opts, k));
    }
    output::spawn_all(&config, &shared, &cancel, &mut tasks);
    source::spawn(&config, source_tx, cancel.clone());

    {
        let c = cancel.clone();
        tokio::spawn(async move {
            service::wait_for_shutdown_signal().await;
            c.cancel();
        });
    }

    if tui {
        let replay = config.connection_type == ConnectionType::File;
        if let Err(e) = ui::run(
            shared.clone(),
            log_buffer,
            cancel.clone(),
            config.tui_refresh_rate_ms,
            replay,
        )
        .await
        {
            eprintln!("TUI error: {e:#}");
        }
        cancel.cancel();
    } else {
        cancel.cancelled().await;
    }

    info!("Shutting down");
    // Let outputs publish their offline status; never hang on exit.
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    info!("Shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses() {
        Cli::command_check();
        let cli = Cli::parse_from(["gps-to-mqtt", "-c", "x.toml", "--headless"]);
        assert_eq!(cli.config.as_deref(), Some("x.toml"));
        assert!(cli.headless);
        assert!(Cli::try_parse_from(["gps-to-mqtt", "--probe"]).is_err());
    }

    impl Cli {
        fn command_check() {
            use clap::CommandFactory;
            Cli::command().debug_assert();
        }
    }
}
