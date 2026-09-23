//! Optional local evidence archive. MQTT outages do not determine retention.
use std::io;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

const MAX_BYTES: u64 = 512 * 1024 * 1024;

async fn append(path: &str, packet: &str) -> io::Result<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    if file.metadata().await?.len() + packet.len() as u64 + 1 > MAX_BYTES {
        return Err(io::Error::other(
            "telemetry archive reached 512 MiB; rotate or export it",
        ));
    }
    file.write_all(format!("{packet}\n").as_bytes()).await?;
    file.sync_data().await
}

pub async fn record(path: Option<&str>, packet: &str) {
    let Some(path) = path else {
        return;
    };
    if let Err(error) = append(path, packet).await {
        // Storage failure must not fill the service journal at sensor frequency.
        static LAST_WARNING: AtomicU64 = AtomicU64::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = LAST_WARNING.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 60
            && LAST_WARNING
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            tracing::error!(%error, "Local telemetry archive unavailable; live capture continues without archiving");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn archive_preserves_packets_across_rotation_and_enforces_cap() {
        let directory = std::env::temp_dir().join(format!(
            "g86-archive-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir(&directory).await.unwrap();
        let path = directory.join("telemetry.ndjson");
        let name = path.to_str().unwrap();
        append(name, r#"{"sequence":1}"#).await.unwrap();
        append(name, r#"{"sequence":2}"#).await.unwrap();
        let saved = directory.join("telemetry.ndjson.1");
        tokio::fs::rename(&path, &saved).await.unwrap();
        append(name, r#"{"sequence":3}"#).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&saved)
                .await
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "{\"sequence\":3}\n"
        );
        let file = OpenOptions::new().write(true).open(&path).await.unwrap();
        file.set_len(MAX_BYTES).await.unwrap();
        assert!(append(name, "{}").await.is_err());
        drop(file);
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }
}
