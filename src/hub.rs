//! The hub: the only consumer of source bytes.
//!
//! `source bytes → LineFramer → Sentence (checksum) → Engine → Fix`
//! then fan-out over the [`Bus`](crate::shared::BusEvent), snapshot update and
//! archive queue. The hub never awaits on an output.

use crate::archive;
use crate::engine::Engine;
use crate::nmea::{LineFramer, NmeaError, Sentence};
use crate::position::PositionEncoder;
use crate::shared::{BusEvent, Counters, LinkStatus, Shared};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Messages from a source task.
#[derive(Debug)]
pub enum SourceEvent {
    Bytes(Vec<u8>),
    Status(LinkStatus, String),
}

/// Pending epochs are flushed after this much silence.
const IDLE_FLUSH: Duration = Duration::from_millis(1500);

pub struct HubOptions {
    pub require_checksum: bool,
    pub telemetry_window: Option<usize>,
    pub satellite_timeout: Duration,
    pub static_hold: Option<crate::hold::HoldConfig>,
    pub archive: Option<mpsc::Sender<Arc<str>>>,
}

pub async fn run(
    mut rx: mpsc::Receiver<SourceEvent>,
    shared: Arc<Shared>,
    opts: HubOptions,
    cancel: CancellationToken,
) {
    let mut hub = Hub {
        framer: LineFramer::new(),
        engine: Engine::new(opts.telemetry_window, opts.satellite_timeout)
            .with_hold(opts.static_hold),
        g86: PositionEncoder::new(),
        shared,
        require_checksum: opts.require_checksum,
        archive: opts.archive,
        sentences_this_second: 0,
        fixes_this_second: 0,
        last_sky_len: usize::MAX,
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            event = rx.recv() => match event {
                Some(SourceEvent::Bytes(bytes)) => hub.bytes(&bytes),
                Some(SourceEvent::Status(status, detail)) => {
                    hub.shared.set_source(status, detail);
                }
                None => break,
            },
            _ = tick.tick() => hub.tick(),
        }
    }
    info!("Hub stopped");
}

struct Hub {
    framer: LineFramer,
    engine: Engine,
    g86: PositionEncoder,
    shared: Arc<Shared>,
    require_checksum: bool,
    archive: Option<mpsc::Sender<Arc<str>>>,
    sentences_this_second: u64,
    fixes_this_second: u64,
    last_sky_len: usize,
}

impl Hub {
    fn bytes(&mut self, bytes: &[u8]) {
        let now = Instant::now();
        self.shared
            .counters
            .bytes
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);

        let mut lines: Vec<String> = Vec::new();
        self.framer.push(bytes, |l| lines.push(l.to_string()));
        if lines.is_empty() {
            return;
        }

        let mut raw = Vec::with_capacity(lines.len());
        let mut fixes = Vec::new();
        let mut texts = Vec::new();
        for line in &lines {
            let sentence = match Sentence::parse(line, self.require_checksum) {
                Ok(s) => s,
                Err(NmeaError::ChecksumMismatch { .. } | NmeaError::BadChecksumFormat) => {
                    Counters::inc(&self.shared.counters.checksum_errors);
                    debug!("checksum error: {line}");
                    continue;
                }
                Err(e) => {
                    Counters::inc(&self.shared.counters.malformed);
                    debug!("rejected ({e}): {line}");
                    continue;
                }
            };
            Counters::inc(&self.shared.counters.sentences);
            self.sentences_this_second += 1;

            let line: Arc<str> = Arc::from(sentence.raw);
            self.shared.publish(BusEvent::Nmea(line.clone()));
            raw.push(line);

            if let Some(packet) = self.g86.sentence(sentence.raw) {
                let packet: Arc<str> = Arc::from(packet);
                if let Some(tx) = &self.archive {
                    archive::record(tx, &self.shared, packet.clone());
                }
                self.shared.publish(BusEvent::G86(packet));
            }

            let out = self.engine.ingest(&sentence, now);
            if let Some(fix) = out.fix {
                fixes.push(Arc::new(fix));
            }
            if let Some(text) = out.text {
                texts.push(text);
            }
        }

        for fix in &fixes {
            self.emit_fix(fix.clone(), now);
        }
        let mut s = self.shared.write();
        s.last_data_at = Some(now);
        if s.source != LinkStatus::Connected && !raw.is_empty() {
            s.source = LinkStatus::Connected;
            s.source_since = now;
        }
        for line in raw {
            s.push_raw(line);
        }
        if let Some(fix) = fixes.last() {
            s.fix = Some(fix.clone());
            s.last_fix_at = Some(now);
        }
        for text in texts {
            info!(target: "receiver", "{text}");
            s.push_message(text);
        }
    }

    fn emit_fix(&mut self, fix: Arc<crate::models::Fix>, _now: Instant) {
        Counters::inc(&self.shared.counters.fixes);
        self.fixes_this_second += 1;
        self.shared.publish(BusEvent::Fix(fix));
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(fix) = self.engine.flush_if_idle(now, IDLE_FLUSH) {
            let fix = Arc::new(fix);
            self.emit_fix(fix.clone(), now);
            let mut s = self.shared.write();
            s.fix = Some(fix);
            s.last_fix_at = Some(now);
        }

        let sky = Arc::new(self.engine.sky(now));
        if sky.in_view > 0 || self.last_sky_len != 0 {
            self.shared.publish(BusEvent::Sky(sky.clone()));
        }
        self.last_sky_len = sky.in_view;

        let mut s = self.shared.write();
        s.sky = sky;
        s.sentence_rate = self.sentences_this_second as f64;
        s.fix_rate = self.fixes_this_second as f64;
        self.sentences_this_second = 0;
        self.fixes_this_second = 0;

        let silent = s
            .last_data_at
            .is_some_and(|t| now.duration_since(t) > Duration::from_secs(5));
        if silent && s.source == LinkStatus::Connected {
            warn!("Source connected but silent for 5 s");
            s.source = LinkStatus::Connecting;
            s.source_since = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmea::with_checksum;
    use crate::shared::Info;

    #[tokio::test]
    async fn bytes_flow_to_bus_snapshot_and_counters() {
        let shared = Shared::new(Info::default(), false);
        let mut bus = shared.bus.subscribe();
        let (tx, rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(
            rx,
            shared.clone(),
            HubOptions {
                require_checksum: true,
                telemetry_window: Some(3),
                satellite_timeout: Duration::from_secs(10),
                static_hold: None,
                archive: None,
            },
            cancel.clone(),
        ));

        let rmc = with_checksum("GNRMC,120000.10,A,5658.0000,N,02414.0000,E,50.0,85.0,230926,,,A");
        let gga = with_checksum("GNGGA,120000.10,5658.0000,N,02414.0000,E,1,12,0.8,12.0,M,0.0,M,,");
        let chunk = format!("{rmc}\r\n{gga}\r\n$GPGGA,bad*00\r\n");
        // Split mid-sentence to exercise framing across reads.
        let (a, b) = chunk.as_bytes().split_at(30);
        tx.send(SourceEvent::Bytes(a.to_vec())).await.unwrap();
        tx.send(SourceEvent::Bytes(b.to_vec())).await.unwrap();

        let mut kinds = Vec::new();
        while kinds.len() < 4 {
            match tokio::time::timeout(Duration::from_secs(2), bus.recv())
                .await
                .unwrap()
                .unwrap()
            {
                BusEvent::Nmea(_) => kinds.push("nmea"),
                BusEvent::G86(p) => {
                    assert!(p.contains("\"schema\":1"));
                    kinds.push("g86");
                }
                BusEvent::Fix(f) => {
                    assert!(f.valid);
                    kinds.push("fix");
                }
                BusEvent::Sky(_) => {}
            }
        }
        assert_eq!(kinds, ["nmea", "nmea", "g86", "fix"]);
        assert_eq!(Counters::get(&shared.counters.sentences), 2);
        assert_eq!(Counters::get(&shared.counters.checksum_errors), 1);
        {
            let s = shared.read();
            assert_eq!(s.source, LinkStatus::Connected);
            assert_eq!(s.raw.len(), 2);
            assert!(s.fix.is_some());
        }
        cancel.cancel();
        task.await.unwrap();
    }
}
