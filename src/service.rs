//! Signal handling for graceful shutdown in service / CLI mode.

use tokio::signal;
use tracing::info;

/// Wait for Ctrl+C or SIGTERM (Unix) then return — used in non-TTY mode.
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
        _ = ctrl_c => { info!("Received Ctrl+C"); },
        _ = terminate => { info!("Received SIGTERM"); },
    }
}

#[cfg(not(unix))]
pub async fn setup_sighup_handler() -> Result<tokio::sync::mpsc::Receiver<()>> {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    Ok(rx)
}
