//! gpsd-compatible JSON server (protocol 3.x subset).
//!
//! Lets any gpsd client — `cgps`, `gpsmon`, `gpspipe`, Navit, FoxtrotGPS,
//! Kismet, Python `gps`/`gpsd-py3`, libgps — use this bridge as if it were gpsd.
//!
//! Supported commands: `?VERSION;`, `?DEVICES;`, `?WATCH;`,
//! `?WATCH={"enable":true,"json":true,"nmea":false};`, `?POLL;`.
//! Reports: `VERSION`, `DEVICES`, `WATCH`, `TPV`, `SKY`, `POLL`, raw NMEA when
//! `"nmea":true`.

use crate::models::{Fix, FixQuality, Sky};
use crate::output::nmea_net::accept_loop;
use crate::shared::{BusEvent, Shared};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::info;

const MAX_COMMAND: usize = 1024;

fn iso(fix: &Fix) -> Option<String> {
    fix.time
        .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn device_path(shared: &Shared) -> String {
    let detail = shared.read().source_detail.clone();
    if detail.is_empty() {
        shared.info.source.clone()
    } else {
        detail
    }
}

/// gpsd TPV `status`.
fn gpsd_status(q: FixQuality) -> u8 {
    match q {
        FixQuality::Invalid => 0,
        FixQuality::Gps | FixQuality::Pps => 1,
        FixQuality::Dgps => 2,
        FixQuality::Rtk => 3,
        FixQuality::FloatRtk => 4,
        FixQuality::Estimated => 5,
        FixQuality::Manual => 7,
        FixQuality::Simulation => 8,
    }
}

pub fn tpv(fix: &Fix, device: &str) -> Value {
    let mut v = json!({"class": "TPV", "device": device, "mode": fix.mode.gpsd()});
    let o = v.as_object_mut().expect("object");
    let mut put = |k: &str, x: Option<Value>| {
        if let Some(x) = x {
            o.insert(k.into(), x);
        }
    };
    put("time", iso(fix).map(Value::from));
    put("status", fix.valid.then(|| json!(gpsd_status(fix.quality))));
    if let Some((lat, lon)) = fix.position() {
        put("lat", Some(json!(lat)));
        put("lon", Some(json!(lon)));
        put("altMSL", fix.altitude_m.map(Value::from));
        put("alt", fix.altitude_m.map(Value::from));
        put(
            "altHAE",
            fix.altitude_m
                .zip(fix.geoid_separation_m)
                .map(|(a, g)| json!(a + g)),
        );
        put("geoidSep", fix.geoid_separation_m.map(Value::from));
        put("eph", fix.accuracy_m.map(Value::from));
        put("epy", fix.error_lat_m.map(Value::from));
        put("epx", fix.error_lon_m.map(Value::from));
        put("epv", fix.error_alt_m.map(Value::from));
    }
    put("speed", fix.speed_mps.map(Value::from));
    put("track", fix.course_deg.map(Value::from));
    put("magvar", fix.magnetic_variation_deg.map(Value::from));
    put("dgpsAge", fix.dgps_age_s.map(Value::from));
    put("dgpsSta", fix.dgps_station.map(Value::from));
    v
}

pub fn sky(sky: &Sky, device: &str) -> Value {
    json!({
        "class": "SKY",
        "device": device,
        "hdop": sky.hdop,
        "vdop": sky.vdop,
        "pdop": sky.pdop,
        "nSat": sky.in_view,
        "uSat": sky.used,
        "satellites": sky.satellites.iter().map(|s| json!({
            "PRN": s.prn,
            "gnssid": s.constellation.gpsd_id(),
            "svid": s.prn,
            "el": s.elevation_deg,
            "az": s.azimuth_deg,
            "ss": s.snr_dbhz,
            "used": s.used,
        })).collect::<Vec<_>>(),
    })
}

fn version() -> Value {
    json!({
        "class": "VERSION",
        "release": "3.25",
        "rev": concat!("gps-to-mqtt ", env!("CARGO_PKG_VERSION")),
        "proto_major": 3,
        "proto_minor": 15,
    })
}

fn devices(shared: &Shared) -> Value {
    json!({
        "class": "DEVICES",
        "devices": [{"class": "DEVICE", "path": device_path(shared), "driver": "NMEA0183", "flags": 1, "native": 0}],
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Watch {
    enable: bool,
    json: bool,
    nmea: bool,
}

impl Watch {
    fn report(&self) -> Value {
        json!({"class": "WATCH", "enable": self.enable, "json": self.json, "nmea": self.nmea, "raw": 0, "scaled": false, "timing": false, "split24": false, "pps": false})
    }

    fn apply(&mut self, args: &Value) {
        let flag = |k: &str| args.get(k).and_then(Value::as_bool);
        if let Some(enable) = flag("enable") {
            self.enable = enable;
            // gpsd defaults to JSON reports when watching without an explicit mode.
            if enable && flag("json").is_none() && flag("nmea").is_none() {
                self.json = true;
            }
        }
        if let Some(j) = flag("json") {
            self.json = j;
        }
        if let Some(n) = flag("nmea") {
            self.nmea = n;
        }
    }
}

/// Process one `?COMMAND[=json]` and return replies.
fn command(cmd: &str, watch: &mut Watch, shared: &Shared) -> Vec<Value> {
    let cmd = cmd.trim().trim_end_matches(';');
    let Some(cmd) = cmd.strip_prefix('?') else {
        return vec![];
    };
    let (name, args) = match cmd.split_once('=') {
        Some((n, a)) => (n, serde_json::from_str::<Value>(a).unwrap_or(Value::Null)),
        None => (cmd, Value::Null),
    };
    match name {
        "VERSION" => vec![version()],
        "DEVICES" => vec![devices(shared)],
        "WATCH" => {
            watch.apply(&args);
            vec![devices(shared), watch.report()]
        }
        "POLL" => {
            let snap = shared.read();
            let device = device_path(shared);
            let tpvs: Vec<Value> = snap.fix.iter().map(|f| tpv(f, &device)).collect();
            vec![json!({
                "class": "POLL",
                "time": snap.fix.as_deref().and_then(iso),
                "active": 1,
                "tpv": tpvs,
                "sky": [sky(&snap.sky, &device)],
            })]
        }
        _ => vec![json!({"class": "ERROR", "message": format!("Unrecognized request '{name}'")})],
    }
}

async fn write_json(stream: &mut (impl AsyncWriteExt + Unpin), v: &Value) -> std::io::Result<()> {
    let mut line = v.to_string();
    line.push_str("\r\n");
    stream.write_all(line.as_bytes()).await
}

async fn serve(mut stream: TcpStream, shared: Arc<Shared>, cancel: CancellationToken) {
    let mut bus = shared.bus.subscribe();
    let (mut rd, mut wr) = stream.split();
    let mut watch = Watch::default();
    let mut pending = Vec::<u8>::new();
    let mut buf = [0u8; 512];
    let device = device_path(&shared);
    if write_json(&mut wr, &version()).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            r = rd.read(&mut buf) => {
                let n = match r { Ok(0) | Err(_) => return, Ok(n) => n };
                pending.extend_from_slice(&buf[..n]);
                if pending.len() > MAX_COMMAND {
                    return; // misbehaving client
                }
                while let Some(end) = pending.iter().position(|b| matches!(b, b';' | b'\n')) {
                    let raw: Vec<u8> = pending.drain(..=end).collect();
                    let text = String::from_utf8_lossy(&raw);
                    for reply in command(&text, &mut watch, &shared) {
                        if write_json(&mut wr, &reply).await.is_err() { return; }
                    }
                }
            }
            ev = bus.recv() => {
                if !watch.enable { continue; }
                let res = match ev {
                    Ok(BusEvent::Fix(f)) if watch.json => write_json(&mut wr, &tpv(&f, &device)).await,
                    Ok(BusEvent::Sky(s)) if watch.json => write_json(&mut wr, &sky(&s, &device)).await,
                    Ok(BusEvent::Nmea(line)) if watch.nmea => {
                        wr.write_all(format!("{line}\r\n").as_bytes()).await
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                    _ => Ok(()),
                };
                if res.is_err() { return; }
            }
        }
    }
}

pub async fn run(
    addr: String,
    max_clients: usize,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding gpsd server to {addr}"))?;
    info!("gpsd-compatible server on tcp://{addr}");
    accept_loop(listener, max_clients, shared, cancel, "gpsd", serve).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AccuracySource, FixMode};
    use crate::shared::Info;
    use chrono::{TimeZone, Utc};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};

    fn fix() -> Fix {
        Fix {
            valid: true,
            mode: FixMode::Fix3D,
            quality: FixQuality::Dgps,
            time: Some(Utc.timestamp_millis_opt(1_790_164_800_100).unwrap()),
            latitude: Some(56.9),
            longitude: Some(24.1),
            altitude_m: Some(10.0),
            geoid_separation_m: Some(20.0),
            speed_mps: Some(3.0),
            accuracy_m: Some(4.0),
            accuracy_source: Some(AccuracySource::HdopEstimate),
            ..Fix::default()
        }
    }

    #[test]
    fn tpv_fields() {
        let v = tpv(&fix(), "/dev/x");
        assert_eq!(v["class"], "TPV");
        assert_eq!(v["mode"], 3);
        assert_eq!(v["status"], 2);
        assert_eq!(v["time"], "2026-09-23T12:00:00.100Z");
        assert_eq!(v["altHAE"], 30.0);
        assert_eq!(v["speed"], 3.0);
        let mut f = fix();
        f.valid = false;
        f.mode = FixMode::NoFix;
        let v = tpv(&f, "/dev/x");
        assert_eq!(v["mode"], 1);
        assert!(v.get("lat").is_none());
    }

    #[test]
    fn watch_command_semantics() {
        let shared = Shared::new(Info::default(), false);
        let mut w = Watch::default();
        let r = command("?WATCH={\"enable\":true};", &mut w, &shared);
        assert_eq!(r.len(), 2);
        assert_eq!(
            w,
            Watch {
                enable: true,
                json: true,
                nmea: false
            }
        );
        command("?WATCH={\"nmea\":true,\"json\":false}", &mut w, &shared);
        assert_eq!(
            w,
            Watch {
                enable: true,
                json: false,
                nmea: true
            }
        );
        assert_eq!(command("?VERSION;", &mut w, &shared)[0]["proto_major"], 3);
        assert_eq!(command("?BOGUS;", &mut w, &shared)[0]["class"], "ERROR");
        assert!(command("garbage", &mut w, &shared).is_empty());
    }

    async fn next(
        lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    ) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn client_session_streams_tpv() {
        let shared = Shared::new(Info::default(), false);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(accept_loop(
            listener,
            4,
            shared.clone(),
            cancel.clone(),
            "gpsd",
            serve,
        ));

        let stream = TcpStream::connect(addr).await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut lines = BufReader::new(rd).lines();
        assert_eq!(next(&mut lines).await["class"], "VERSION");
        wr.write_all(b"?WATCH={\"enable\":true,\"json\":true};\n")
            .await
            .unwrap();
        assert_eq!(next(&mut lines).await["class"], "DEVICES");
        assert_eq!(next(&mut lines).await["class"], "WATCH");
        shared.publish(BusEvent::Fix(Arc::new(fix())));
        let tpv = next(&mut lines).await;
        assert_eq!(tpv["class"], "TPV");
        assert_eq!(tpv["lat"], 56.9);
        cancel.cancel();
    }
}
