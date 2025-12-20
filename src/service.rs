use anyhow::Result;
use tokio::signal;
use tracing::{info, warn};

/// Handle Unix signals for graceful shutdown
pub async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C signal");
        },
        _ = terminate => {
            info!("Received SIGTERM signal");
        },
    }
}

/// Set up a signal handler for SIGHUP to support configuration reload in daemon mode
#[allow(dead_code)]
pub async fn setup_sighup_handler() -> Result<tokio::sync::mpsc::Receiver<()>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    tokio::spawn(async move {
        let mut stream = signal::unix::signal(signal::unix::SignalKind::hangup())
            .expect("Failed to install SIGHUP handler");

        while stream.recv().await.is_some() {
            info!("Received SIGHUP signal");
            if tx.send(()).await.is_err() {
                warn!("SIGHUP receiver dropped");
                break;
            }
        }
    });

    Ok(rx)
}

#[cfg(not(unix))]
pub async fn setup_sighup_handler() -> Result<tokio::sync::mpsc::Receiver<()>> {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    Ok(rx)
}
