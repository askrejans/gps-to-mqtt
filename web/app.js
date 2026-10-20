/* gps-to-mqtt web dashboard.
 *
 * Data: GET api/v1/{status,fix,sky,config,nmea} + the api/v1/stream SSE feed,
 * read with fetch() so the bearer token travels in a header (EventSource
 * cannot send one). All dynamic text is set with textContent: receiver
 * messages and NMEA are untrusted input. Only the visible mode is redrawn.
 */
"use strict";
(() => {
  const $ = (id) => document.getElementById(id);
  const store = {
    get(k, d) { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch { /* storage blocked */ } },
  };
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const C = {
    display: "#b4f3e6", cyan: "#5cf0d6", amber: "#ffb43c", green: "#45f5a1", red: "#ff5544",
    text: "#cfd5d7", muted: "#7f878b", faint: "#3a4246", grid: "rgba(190,200,205,0.10)", gridHi: "rgba(190,200,205,0.22)",
    font: '"Chakra Petch", ui-sans-serif, system-ui, sans-serif', num: '"Doto", "JetBrains Mono", ui-monospace, monospace',
  };
  const SYS = {
    gps: ["G", "#45f5a1", "GPS"], glonass: ["R", "#5aa9ff", "GLONASS"], galileo: ["E", "#5cf0d6", "Galileo"],
    beidou: ["C", "#e7a6ff", "BeiDou"], qzss: ["J", "#ff8a7a", "QZSS"], sbas: ["S", "#ffb43c", "SBAS"],
    navic: ["I", "#9fd0ff", "NavIC"], unknown: ["?", "#8d959c", "Other"],
  };
  const UNITS = {
    kmh: { label: "km/h", f: 3.6, alt: "m", altF: 1, dist: "km", distF: 0.001 },
    mph: { label: "mph", f: 2.2369363, alt: "ft", altF: 3.2808399, dist: "mi", distF: 0.000621371 },
    kn: { label: "kn", f: 1.9438445, alt: "m", altF: 1, dist: "NM", distF: 0.000539957 },
  };
  const MODES = ["drive", "nav", "sky", "system", "config"];

  const S = {
    fix: null, sky: null, status: null, config: null,
    mode: MODES.includes(location.hash.slice(1)) ? location.hash.slice(1) : store.get("mode", "drive"),
    units: UNITS[store.get("units")] ? store.get("units") : "kmh",
    dim: store.get("dim") === "1",
    token: store.get("token", ""),
    trail: [], gg: [], peak: 0,
    view: store.get("trackView", "map"), zoom: Number(store.get("mapZoom", 17)) || 17,
    follow: true, center: null,
    lastFixAt: 0, streamUp: false, nmeaPaused: false,
    prevCounters: null, prevCountersAt: 0, rates: {},
  };

  // ── formatting ────────────────────────────────────────────────────────
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const fmt = (v, d = 1) => (isNum(v) ? v.toFixed(d) : "—");
  const signed = (v, d = 2) => (isNum(v) ? (v >= 0 ? "+" : "−") + Math.abs(v).toFixed(d) : "—");
  const U = () => UNITS[S.units];
  const speed = (mps, d = 1) => (isNum(mps) ? (mps * U().f).toFixed(d) : "—");
  const dms = (deg, pos, neg) => {
    if (!isNum(deg)) return "—";
    const a = Math.abs(deg), d = Math.floor(a), mf = (a - d) * 60, m = Math.floor(mf), s = (mf - m) * 60;
    return `${d}°${String(m).padStart(2, "0")}′${s.toFixed(2).padStart(5, "0")}″${deg >= 0 ? pos : neg}`;
  };
  const pad2 = (n) => String(n).padStart(2, "0");
  /** Fixed-width UTC time. chrono omits ".0" on whole seconds, so never slice the ISO string. */
  function utc(iso, tenths = true) {
    const d = iso ? new Date(iso) : null;
    if (!d || Number.isNaN(d.getTime())) return tenths ? "--:--:--.-" : "--:--:--";
    const t = `${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}:${pad2(d.getUTCSeconds())}`;
    return tenths ? `${t}.${Math.floor(d.getUTCMilliseconds() / 100)}` : t;
  }
  const utcDate = (iso) => {
    const d = iso ? new Date(iso) : null;
    return d && !Number.isNaN(d.getTime())
      ? `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())} ${utc(iso)}`
      : "—";
  };
  const ACC_SRC = { gst: "receiver σ (GST)", receiver: "receiver estimate", "hdop-estimate": "HDOP estimate" };
  const QUALITY = { invalid: "Invalid", gps: "GPS", dgps: "DGPS", pps: "PPS", rtk: "RTK fixed", float_rtk: "RTK float",
    estimated: "Dead reckoning", manual: "Manual", simulation: "Simulation" };
  const fixAge = () => (S.lastFixAt ? (performance.now() - S.lastFixAt) / 1000 : Infinity);

  function badge(fix) {
    if (!fix) return ["NO DATA", ""];
    if (!fix.valid) return ["NO FIX", "bad"];
    if (fixAge() > 3) return ["STALE", "warn"];
    if (fix.quality === "rtk") return ["RTK FIXED", "rtk"];
    if (fix.quality === "float_rtk") return ["RTK FLOAT", "rtk"];
    if (fix.quality === "estimated") return ["DEAD RECK.", "warn"];
    const dim = fix.mode === "3d" ? "3D" : fix.mode === "2d" ? "2D" : "";
    return [`${dim} ${fix.quality === "dgps" ? "DGPS" : "FIX"}`.trim(), "ok"];
  }

  // ── DOM helpers ───────────────────────────────────────────────────────
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function setText(id, v) { const e = $(id); if (e && e.textContent !== v) e.textContent = v; }
  /** Replace a <dl> with [label, value, class?, small?] rows. */
  function kv(id, rows) {
    const dl = $(id);
    const frag = document.createDocumentFragment();
    for (const [k, v, cls, small] of rows) {
      frag.append(el("dt", null, k));
      const dd = el("dd", cls || null, v);
      if (small) dd.append(el("small", null, small));
      frag.append(dd);
    }
    dl.replaceChildren(frag);
  }
  function segs(id, n) {
    const e = $(id);
    if (e.children.length !== n) e.replaceChildren(...Array.from({ length: n }, () => el("i")));
    return e.children;
  }
  /** Unipolar meter: fraction 0..1, colour zones by position. */
  function meter(id, frac, n = 24, zone = (i) => (i < n * 0.6 ? "" : i < n * 0.85 ? "a" : "r")) {
    const s = segs(id, n), lit = Math.round(Math.max(0, Math.min(1, frac)) * n);
    for (let i = 0; i < n; i++) s[i].className = i < lit ? `on ${zone(i)}` : "";
  }
  /** Centre-zero meter: value -1..1. */
  function bipolar(id, v, n = 25) {
    const s = segs(id, n), c = (n - 1) / 2, reach = Math.round(Math.min(1, Math.abs(v || 0)) * c);
    for (let i = 0; i < n; i++) {
      const d = i - c, lit = isNum(v) && d !== 0 && Math.sign(d) === Math.sign(v) && Math.abs(d) <= reach;
      const mag = Math.abs(d) / c;
      s[i].className = i === c ? "mid" : lit ? `on ${mag > 0.66 ? "r" : mag > 0.4 ? "a" : ""}` : "";
    }
  }

  // ── canvas helpers ────────────────────────────────────────────────────
  function canvas(id) {
    const cv = $(id), dpr = window.devicePixelRatio || 1;
    const w = cv.clientWidth, h = cv.clientHeight;
    if (!w || !h) return null;
    if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
      cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr);
    }
    const ctx = cv.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    return { ctx, w, h };
  }
  function ring(ctx, x, y, r, color, width = 1) {
    ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.strokeStyle = color; ctx.lineWidth = width; ctx.stroke();
  }
  function label(ctx, text, x, y, color = C.muted, size = 11, align = "center", font = C.font) {
    ctx.fillStyle = color; ctx.font = `${size}px ${font}`; ctx.textAlign = align; ctx.textBaseline = "middle";
    ctx.fillText(text, x, y);
  }
  function glowDot(ctx, x, y, r, color, filled = true) {
    ctx.save(); ctx.shadowColor = color; ctx.shadowBlur = filled ? 8 : 0;
    ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2);
    if (filled) { ctx.fillStyle = color; ctx.fill(); } else { ctx.strokeStyle = color; ctx.lineWidth = 1.5; ctx.stroke(); }
    ctx.restore();
  }

  // ── API ───────────────────────────────────────────────────────────────
  const headers = () => (S.token ? { Authorization: `Bearer ${S.token}` } : {});
  async function api(path) {
    const res = await fetch(path, { headers: headers(), cache: "no-store" });
    if (res.status === 401) { askToken(); throw new Error("unauthorized"); }
    return res;
  }
  function askToken() { const d = $("auth"); if (!d.open) d.showModal(); }
  $("auth").addEventListener("close", () => {
    const t = $("auth-token").value.trim();
    if (!t) return;
    S.token = t; store.set("token", t); $("auth-token").value = "";
    restartStream(); loadOnce(); pollStatus();
  });

  let streamCtl = null;
  async function stream() {
    let delay = 1000;
    for (;;) {
      const ctl = new AbortController();
      streamCtl = ctl;
      try {
        const res = await fetch("api/v1/stream", { headers: { ...headers(), Accept: "text/event-stream" }, signal: ctl.signal, cache: "no-store" });
        if (res.status === 401) { setStream(false); askToken(); return; }
        if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
        setStream(true); delay = 1000;
        const reader = res.body.getReader(), dec = new TextDecoder();
        let buf = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true }).replace(/\r\n/g, "\n");
          let i;
          while ((i = buf.indexOf("\n\n")) >= 0) { dispatch(buf.slice(0, i)); buf = buf.slice(i + 2); }
        }
      } catch (e) {
        if (ctl.signal.aborted) return;
      }
      setStream(false);
      await sleep(delay); delay = Math.min(delay * 2, 15000);
    }
  }
  function restartStream() { if (streamCtl) streamCtl.abort(); stream(); }
  function dispatch(block) {
    let ev = "message", data = "";
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) ev = line.slice(6).trim();
      else if (line.startsWith("data:")) data += line.slice(5).trimStart();
    }
    if (!data) return;
    try {
      if (ev === "fix") onFix(JSON.parse(data));
      else if (ev === "sky") onSky(JSON.parse(data));
    } catch { /* malformed event */ }
  }
  function setStream(up) { S.streamUp = up; schedule(); }

  function onFix(fix) {
    S.fix = fix; S.lastFixAt = performance.now();
    if (fix.valid && isNum(fix.latitude)) {
      const last = S.trail[S.trail.length - 1];
      if (!last || last[0] !== fix.latitude || last[1] !== fix.longitude) {
        S.trail.push([fix.latitude, fix.longitude]);
        if (S.trail.length > 1200) S.trail.shift();
      }
    }
    const t = fix.telemetry;
    if (t && isNum(t.lateral_g) && isNum(t.longitudinal_g)) {
      S.gg.push([t.lateral_g, t.longitudinal_g, performance.now()]);
      if (S.gg.length > 120) S.gg.shift();
      if (isNum(t.combined_g)) S.peak = Math.max(S.peak, t.combined_g);
    }
    schedule();
  }
  function onSky(sky) { S.sky = sky; schedule(); }

  async function loadOnce() {
    try { const r = await api("api/v1/config"); if (r.ok) S.config = await r.json(); } catch { /* retried by poll */ }
    try { const r = await api("api/v1/fix"); if (r.ok && !S.fix) onFix(await r.json()); } catch { /* none yet */ }
    try { const r = await api("api/v1/sky"); if (r.ok && !S.sky) onSky(await r.json()); } catch { /* none yet */ }
    schedule();
  }

  async function pollStatus() {
    try {
      const r = await api("api/v1/status");
      if (r.ok) {
        const st = await r.json(), now = performance.now();
        if (S.prevCounters) {
          const dt = (now - S.prevCountersAt) / 1000;
          for (const [k, v] of Object.entries(st.counters)) S.rates[k] = (v - (S.prevCounters[k] ?? v)) / dt;
        }
        S.prevCounters = st.counters; S.prevCountersAt = now; S.status = st;
        if (!S.config) loadOnce();
      }
    } catch { S.status = S.status && { ...S.status, stale: true }; }
    schedule();
  }

  async function pollNmea() {
    if (S.mode !== "system" || S.nmeaPaused || document.hidden) return;
    try {
      const r = await api("api/v1/nmea?limit=200");
      if (r.ok) renderNmea(await r.text());
    } catch { /* shown by lamps */ }
  }

  // ── render scheduling ─────────────────────────────────────────────────
  let queued = false;
  function schedule() { if (!queued) { queued = true; requestAnimationFrame(render); } }
  function render() {
    queued = false;
    renderBar(); renderReadout();
    ({ drive: renderDrive, nav: renderNav, sky: renderSky, system: renderSystem, config: renderConfig })[S.mode]();
  }

  function lamp(id, state) { $(id).className = `lamp ${state}`; }
  function renderBar() {
    const st = S.status, fix = S.fix, h = st?.health;
    setText("version", st?.info?.version ? `v${st.info.version}` : "");
    lamp("lamp-source", !h ? "" : h.source === "connected" ? "ok" : h.source === "connecting" ? "warn" : "bad");
    lamp("lamp-fix", !fix ? "" : !fix.valid ? "bad" : fixAge() > 3 ? "warn" : "ok");
    lamp("lamp-mqtt", !h || h.mqtt === "disabled" ? "" : h.mqtt === "connected" ? "ok" : h.mqtt === "connecting" ? "warn" : "bad");
    lamp("lamp-link", S.streamUp ? "ok" : "bad");
    setText("utc", utc(fix?.time));
    setText("foot-source", h ? `${h.source_detail || st.info.source} · ${st.info.receiver}` : "connecting…");
  }

  function renderReadout() {
    const f = S.fix, h = S.status?.health;
    document.querySelector(".readout").classList.toggle("stale", !f || fixAge() > 3);
    setText("r-speed", speed(f?.speed_mps));
    setText("r-speed-u", U().label);
    setText("r-speed-sub", isNum(f?.speed_mps) ? `${f.speed_mps.toFixed(2)} m/s` : " ");
    const [b, cls] = badge(f);
    const fe = $("r-fix"); fe.textContent = b; fe.className = `val txt ${cls}`;
    setText("r-fix-sub", f ? `${QUALITY[f.quality] || f.quality}${f.position_held ? " · held" : ""} · #${f.sequence}` : " ");
    setText("r-sats", f?.satellites_used != null ? String(f.satellites_used) : "—");
    setText("r-sats-view", `/ ${S.sky?.in_view || f?.satellites_in_view || "—"}`);
    setText("r-acc", f?.valid ? fmt(f.accuracy_m, 1) : "—");
    setText("r-acc-sub", f?.accuracy_source ? ACC_SRC[f.accuracy_source] || f.accuracy_source : " ");
    setText("r-alt", f?.valid && isNum(f.altitude_m) ? (f.altitude_m * U().altF).toFixed(1) : "—");
    setText("r-alt-u", U().alt);
    const moving = isNum(f?.speed_mps) && f.speed_mps * 3.6 >= 3;
    setText("r-crs", moving ? fmt(f.course_deg, 1) : "—");
    setText("r-crs-sub", isNum(f?.heading_deg) ? `heading ${f.heading_deg.toFixed(1)}°` : moving ? "over ground" : "stationary");
    setText("r-rate", h ? fmt(h.fix_rate_hz, 0) : "—");
    setText("r-rate-sub", h ? `${fmt(h.sentence_rate_hz, 0)} sentences/s` : " ");
  }

  // ── Drive ─────────────────────────────────────────────────────────────
  function renderDrive() {
    const t = S.fix?.telemetry;
    setText("d-long", t ? `${signed(t.longitudinal_g)} g` : "—");
    setText("d-lat", t ? `${signed(t.lateral_g)} g` : "—");
    setText("d-comb", t ? `${fmt(t.combined_g, 2)} g` : "—");
    setText("d-yaw", t ? `${signed(t.yaw_rate_dps, 1)}°/s` : "—");
    bipolar("m-long", t?.longitudinal_g != null ? t.longitudinal_g / 1.2 : null);
    bipolar("m-lat", t?.lateral_g != null ? t.lateral_g / 1.2 : null);
    bipolar("m-yaw", t?.yaw_rate_dps != null ? t.yaw_rate_dps / 60 : null);
    meter("m-comb", (t?.combined_g || 0) / 1.5);
    const br = $("d-brake");
    br.textContent = !t ? "—" : t.braking ? "BRAKING" : "OFF";
    br.className = `state ${t?.braking ? "on" : ""}`;
    setText("d-max", t ? `${speed(t.max_speed_mps, 0)} ${U().label}` : "—");
    setText("d-dist", t ? `${(t.distance_m * U().distF).toFixed(2)} ${U().dist}` : "—");
    setText("d-peak", S.peak ? `${S.peak.toFixed(2)} g` : "—");
    drawGG(t); drawCompass();
  }

  function drawGG(t) {
    const c = canvas("c-gg"); if (!c) return;
    const { ctx, w, h } = c, cx = w / 2, cy = h / 2, R = Math.min(w, h) / 2 - 18, k = R / 1.5;
    ctx.strokeStyle = C.grid; ctx.lineWidth = 1;
    ctx.beginPath(); ctx.moveTo(cx - R, cy); ctx.lineTo(cx + R, cy); ctx.moveTo(cx, cy - R); ctx.lineTo(cx, cy + R); ctx.stroke();
    for (const g of [0.5, 1, 1.5]) {
      ring(ctx, cx, cy, g * k, g === 1 ? C.gridHi : C.grid);
      label(ctx, `${g} g`, cx + g * k * 0.707 + 4, cy - g * k * 0.707 - 6, C.muted, 10, "left");
    }
    label(ctx, "ACCEL", cx, cy - R - 8, C.muted, 9); label(ctx, "BRAKE", cx, cy + R + 9, C.muted, 9);
    label(ctx, "L", cx - R - 9, cy, C.muted, 9); label(ctx, "R", cx + R + 9, cy, C.muted, 9);
    const now = performance.now();
    S.gg.forEach(([x, y, at], i) => {
      const age = (now - at) / 3000;
      if (age > 1) return;
      ctx.globalAlpha = 0.65 * (1 - age);
      glowDot(ctx, cx + x * k, cy - y * k, 2, C.cyan);
      if (i === S.gg.length - 1) ctx.globalAlpha = 1;
    });
    ctx.globalAlpha = 1;
    if (t && isNum(t.lateral_g) && isNum(t.longitudinal_g)) {
      const x = cx + Math.max(-1.6, Math.min(1.6, t.lateral_g)) * k, y = cy - Math.max(-1.6, Math.min(1.6, t.longitudinal_g)) * k;
      const col = t.braking ? C.red : C.green;
      ctx.strokeStyle = col; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(cx, cy); ctx.lineTo(x, y); ctx.stroke();
      glowDot(ctx, x, y, 6, col);
    } else {
      label(ctx, t ? "stationary — no dynamics" : "telemetry disabled", cx, cy + 20, C.faint, 12);
    }
  }

  function drawCompass() {
    const c = canvas("c-compass"); if (!c) return;
    const { ctx, w, h } = c, cx = w / 2, cy = h / 2, R = Math.min(w, h) / 2 - 22, f = S.fix;
    ring(ctx, cx, cy, R, C.gridHi);
    for (let d = 0; d < 360; d += 10) {
      const a = (d - 90) * Math.PI / 180, long = d % 30 === 0;
      ctx.strokeStyle = long ? C.gridHi : C.grid; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(cx + Math.cos(a) * (R - (long ? 10 : 5)), cy + Math.sin(a) * (R - (long ? 10 : 5)));
      ctx.lineTo(cx + Math.cos(a) * R, cy + Math.sin(a) * R); ctx.stroke();
    }
    for (const [t, d] of [["N", 0], ["E", 90], ["S", 180], ["W", 270]]) {
      const a = (d - 90) * Math.PI / 180;
      label(ctx, t, cx + Math.cos(a) * (R + 12), cy + Math.sin(a) * (R + 12), t === "N" ? C.text : C.muted, 12);
    }
    const arrow = (deg, color, len, width) => {
      const a = (deg - 90) * Math.PI / 180;
      ctx.save(); ctx.shadowColor = color; ctx.shadowBlur = 6; ctx.strokeStyle = color; ctx.lineWidth = width;
      ctx.beginPath(); ctx.moveTo(cx, cy); ctx.lineTo(cx + Math.cos(a) * len, cy + Math.sin(a) * len); ctx.stroke();
      const tx = cx + Math.cos(a) * len, ty = cy + Math.sin(a) * len;
      ctx.fillStyle = color; ctx.beginPath(); ctx.moveTo(tx, ty);
      ctx.lineTo(tx - Math.cos(a - 0.4) * 12, ty - Math.sin(a - 0.4) * 12); ctx.lineTo(tx - Math.cos(a + 0.4) * 12, ty - Math.sin(a + 0.4) * 12);
      ctx.fill(); ctx.restore();
    };
    const moving = isNum(f?.speed_mps) && f.speed_mps * 3.6 >= 3;
    if (isNum(f?.heading_deg)) arrow(f.heading_deg, C.amber, R - 18, 2);
    if (moving && isNum(f.course_deg)) arrow(f.course_deg, C.cyan, R - 6, 3);
    const main = moving && isNum(f.course_deg) ? f.course_deg : f?.heading_deg;
    label(ctx, isNum(main) ? `${main.toFixed(0)}°` : "—", cx, cy + R * 0.42, C.display, 22, "center", C.num);
    label(ctx, isNum(f?.heading_deg) ? "course · heading" : moving ? "course over ground" : "stationary", cx, cy + R * 0.42 + 20, C.muted, 10);
  }

  // ── Navigation ────────────────────────────────────────────────────────
  function renderNav() {
    const f = S.fix, v = f?.valid, u = U();
    const ell = v && isNum(f.altitude_m) && isNum(f.geoid_separation_m) ? f.altitude_m + f.geoid_separation_m : null;
    kv("nav-pos", [
      ["Latitude", v ? f.latitude.toFixed(7) + "°" : "—", "hl", v ? dms(f.latitude, "N", "S") : ""],
      ["Longitude", v ? f.longitude.toFixed(7) + "°" : "—", "hl", v ? dms(f.longitude, "E", "W") : ""],
      ["MGRS", v && f.mgrs ? f.mgrs : "—", "hl", v && f.mgrs ? "10-digit · 1 m" : v ? "outside 80°S–84°N" : ""],
      ["MGRS 8", v && f.mgrs_8 ? f.mgrs_8 : "—", null, v && f.mgrs_8 ? "8-digit · 10 m" : ""],
      ["Altitude MSL", v && isNum(f.altitude_m) ? `${(f.altitude_m * u.altF).toFixed(1)} ${u.alt}` : "—"],
      ["Ellipsoidal", isNum(ell) ? `${(ell * u.altF).toFixed(1)} ${u.alt}` : "—", null, "WGS-84"],
      ["Geoid sep.", isNum(f?.geoid_separation_m) ? `${f.geoid_separation_m.toFixed(1)} m` : "—"],
      ["Speed", isNum(f?.speed_mps) ? `${speed(f.speed_mps, 2)} ${u.label}` : "—", null,
        isNum(f?.speed_mps) ? `${(f.speed_mps * 3.6).toFixed(1)} km/h · ${(f.speed_mps * 1.9438445).toFixed(1)} kn` : ""],
      ["Course", isNum(f?.course_deg) ? `${f.course_deg.toFixed(1)}°` : "—", null, "true"],
      ["Heading", isNum(f?.heading_deg) ? `${f.heading_deg.toFixed(1)}°` : "—", null, isNum(f?.heading_deg) ? "HDT/THS" : "no heading sensor"],
      ["Mag. variation", isNum(f?.magnetic_variation_deg) ? `${signed(f.magnetic_variation_deg, 1)}°` : "—"],
      ["Rate of turn", isNum(f?.rate_of_turn_dps) ? `${signed(f.rate_of_turn_dps, 2)}°/s` : "—"],
      ["UTC", utcDate(f?.time)],
    ]);
    const age = fixAge();
    kv("nav-quality", [
      ["Valid", f ? (f.valid ? "yes" : "no") : "—", f ? (f.valid ? "ok" : "bad") : null],
      ["Mode", f ? { none: "no fix", "2d": "2D", "3d": "3D" }[f.mode] : "—"],
      ["Quality", f ? QUALITY[f.quality] || f.quality : "—"],
      ["Satellites", f?.satellites_used != null ? `${f.satellites_used} used` : "—", null, S.sky ? `${S.sky.in_view} in view` : ""],
      ["HDOP / VDOP", f ? `${fmt(f.hdop, 2)} / ${fmt(f.vdop, 2)}` : "—"],
      ["PDOP", fmt(f?.pdop, 2)],
      ["Accuracy", f?.valid && isNum(f.accuracy_m) ? `±${f.accuracy_m.toFixed(1)} m` : "—", null, f?.accuracy_source ? ACC_SRC[f.accuracy_source] : ""],
      ["σ lat / lon", isNum(f?.error_lat_m) ? `${fmt(f.error_lat_m, 2)} / ${fmt(f.error_lon_m, 2)} m` : "—", null, "GST"],
      ["σ altitude", isNum(f?.error_alt_m) ? `${fmt(f.error_alt_m, 2)} m` : "—"],
      ["DGPS", isNum(f?.dgps_age_s) ? `${f.dgps_age_s.toFixed(1)} s` : "—", null, f?.dgps_station != null ? `station ${f.dgps_station}` : ""],
      ["Position hold", !S.config?.static_hold ? "disabled" : f?.position_held ? "holding" : "released",
        f?.position_held ? "ok" : null,
        S.config?.static_hold ? `< ${S.config.static_hold_speed_kph} km/h · ${S.config.static_hold_radius_m} m radius` : "static_hold = false"],
      ["Fix age", isFinite(age) ? `${age.toFixed(1)} s` : "—", age > 3 ? "warn" : null],
      ["Sequence", f ? String(f.sequence) : "—"],
    ]);
    drawTrail();
  }

  function niceStep(span) {
    const raw = span / 4, p = Math.pow(10, Math.floor(Math.log10(raw))), n = raw / p;
    return (n < 1.5 ? 1 : n < 3.5 ? 2 : n < 7.5 ? 5 : 10) * p;
  }
  // ── Track: map or local plot ──────────────────────────────────────────
  const mapEnabled = () => Boolean(S.config?.http_map_tiles);
  function drawTrail() {
    const useMap = mapEnabled() && S.view === "map";
    $("k-view-map").hidden = !mapEnabled();
    $("k-view-plot").hidden = !mapEnabled();
    for (const id of ["k-zoom-in", "k-zoom-out", "k-follow"]) $(id).hidden = !useMap;
    $("k-view-map").classList.toggle("active", useMap);
    $("k-view-plot").classList.toggle("active", !useMap);
    $("k-follow").classList.toggle("active", S.follow);
    $("map-attrib").hidden = !useMap;
    $("c-tiles").hidden = !useMap;
    if (useMap) drawMap(); else drawPlot();
  }

  // Web Mercator, 256 px tiles
  const TILE = 256;
  const worldPx = (z) => TILE * 2 ** z;
  const project = (lat, lon, z) => {
    const s = Math.sin(Math.max(-85.0511, Math.min(85.0511, lat)) * Math.PI / 180);
    return [(lon + 180) / 360 * worldPx(z), (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * worldPx(z)];
  };
  const unproject = (x, y, z) => {
    const n = Math.PI - 2 * Math.PI * y / worldPx(z);
    return [180 / Math.PI * Math.atan(Math.sinh(n)), x / worldPx(z) * 360 - 180];
  };
  const metresPerPx = (lat, z) => 156543.03392 * Math.cos(lat * Math.PI / 180) / 2 ** z;

  const tiles = new Map(); // "z/x/y" → HTMLImageElement, insertion-ordered for eviction
  function tile(z, x, y) {
    const key = `${z}/${x}/${y}`;
    let img = tiles.get(key);
    if (img) { tiles.delete(key); tiles.set(key, img); return img; }
    img = new Image();
    img.decoding = "async";
    img.referrerPolicy = "origin"; // OSM tile policy expects an identifying Referer
    img.onload = schedule;
    img.src = S.config.http_map_tiles.replace("{z}", z).replace("{x}", x).replace("{y}", y);
    tiles.set(key, img);
    while (tiles.size > 400) tiles.delete(tiles.keys().next().value);
    return img;
  }

  function drawMap() {
    const tc = canvas("c-tiles"), oc = canvas("c-trail");
    if (!tc || !oc) return;
    $("c-tiles").classList.toggle("dark", S.config.http_map_dark !== false);
    setText("map-attrib", S.config.http_map_attribution || "© OpenStreetMap contributors");
    const { w, h } = oc, z = S.zoom, f = S.fix;
    const here = f?.valid ? [f.latitude, f.longitude] : S.trail[S.trail.length - 1];
    if (S.follow || !S.center) S.center = here || S.center;
    if (!S.center) {
      label(oc.ctx, "waiting for a valid fix", w / 2, h / 2, C.muted, 13);
      setText("trail-note", "map");
      return;
    }
    const [cx, cy] = project(S.center[0], S.center[1], z), ox = cx - w / 2, oy = cy - h / 2, n = 2 ** z;
    const t = tc.ctx;
    t.fillStyle = "#d8dcdf"; t.fillRect(0, 0, w, h); // inverted to near-black when dark
    for (let ty = Math.floor(oy / TILE); ty <= Math.floor((oy + h) / TILE); ty++) {
      if (ty < 0 || ty >= n) continue;
      for (let tx = Math.floor(ox / TILE); tx <= Math.floor((ox + w) / TILE); tx++) {
        const img = tile(z, ((tx % n) + n) % n, ty);
        if (img.complete && img.naturalWidth) t.drawImage(img, Math.round(tx * TILE - ox), Math.round(ty * TILE - oy), TILE, TILE);
      }
    }
    const ctx = oc.ctx, px = ([la, lo]) => { const [x, y] = project(la, lo, z); return [x - ox, y - oy]; };
    ctx.lineWidth = 3; ctx.lineJoin = "round"; ctx.lineCap = "round";
    const pts = S.trail.map(px);
    for (let i = 1; i < pts.length; i++) {
      ctx.strokeStyle = `rgba(92,240,214,${0.25 + 0.75 * i / pts.length})`;
      ctx.beginPath(); ctx.moveTo(pts[i - 1][0], pts[i - 1][1]); ctx.lineTo(pts[i][0], pts[i][1]); ctx.stroke();
    }
    if (here) {
      const [x, y] = px(here);
      if (f?.valid && isNum(f.accuracy_m)) {
        const r = f.accuracy_m / metresPerPx(here[0], z);
        ctx.fillStyle = "rgba(255,180,60,0.10)"; ctx.beginPath(); ctx.arc(x, y, Math.max(4, r), 0, Math.PI * 2); ctx.fill();
        ring(ctx, x, y, Math.max(4, r), "rgba(255,180,60,0.6)");
      }
      if (isNum(f?.course_deg) && isNum(f?.speed_mps) && f.speed_mps * 3.6 >= 3) {
        const a = (f.course_deg - 90) * Math.PI / 180;
        ctx.strokeStyle = C.amber; ctx.lineWidth = 2;
        ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(x + Math.cos(a) * 30, y + Math.sin(a) * 30); ctx.stroke();
      }
      glowDot(ctx, x, y, 6, C.display);
      ctx.strokeStyle = "#041114"; ctx.lineWidth = 1.5; ctx.beginPath(); ctx.arc(x, y, 6, 0, Math.PI * 2); ctx.stroke();
    }
    // scale bar
    const mpp = metresPerPx(S.center[0], z), target = mpp * 110;
    const p = 10 ** Math.floor(Math.log10(target)), step = [1, 2, 5, 10].map((k) => k * p).filter((v) => v <= target).pop() || p;
    const bar = step / mpp, bx = 12, by = h - 14;
    ctx.fillStyle = "rgba(8,9,10,0.7)"; ctx.fillRect(bx - 6, by - 20, bar + 12, 28);
    ctx.strokeStyle = C.text; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(bx, by); ctx.lineTo(bx + bar, by); ctx.stroke();
    label(ctx, step >= 1000 ? `${step / 1000} km` : `${step} m`, bx + bar / 2, by - 10, C.text, 11);
    setText("trail-note", `${S.trail.length} positions · zoom ${z}${S.follow ? " · following" : ""}`);
  }

  function setZoom(z) {
    S.zoom = Math.max(3, Math.min(19, z)); store.set("mapZoom", String(S.zoom)); schedule();
  }

  function drawPlot() {
    const c = canvas("c-trail"); if (!c) return;
    const { ctx, w, h } = c, pts = S.trail;
    if (pts.length === 0) { label(ctx, "waiting for a valid fix", w / 2, h / 2, C.faint, 13); return; }
    const [lat0, lon0] = pts[pts.length - 1];
    const kx = 111320 * Math.cos(lat0 * Math.PI / 180), ky = 110540;
    const xy = pts.map(([la, lo]) => [(lo - lon0) * kx, (la - lat0) * ky]);
    let ext = 10;
    for (const [x, y] of xy) ext = Math.max(ext, Math.abs(x), Math.abs(y));
    ext *= 1.15;
    const pad = 20, s = Math.min(w, h) / 2 / ext * (1 - pad / Math.min(w, h));
    const cx = w / 2, cy = h / 2, step = niceStep(ext * 2);
    ctx.strokeStyle = C.grid; ctx.lineWidth = 1; ctx.beginPath();
    for (let g = -Math.ceil(w / 2 / s / step) * step; g * s <= w / 2; g += step) { ctx.moveTo(cx + g * s, 0); ctx.lineTo(cx + g * s, h); }
    for (let g = -Math.ceil(h / 2 / s / step) * step; g * s <= h / 2; g += step) { ctx.moveTo(0, cy - g * s); ctx.lineTo(w, cy - g * s); }
    ctx.stroke();
    ctx.lineWidth = 2; ctx.lineJoin = "round";
    for (let i = 1; i < xy.length; i++) {
      ctx.strokeStyle = `rgba(92,240,214,${0.15 + 0.85 * i / xy.length})`;
      ctx.beginPath(); ctx.moveTo(cx + xy[i - 1][0] * s, cy - xy[i - 1][1] * s); ctx.lineTo(cx + xy[i][0] * s, cy - xy[i][1] * s); ctx.stroke();
    }
    const f = S.fix;
    if (isNum(f?.course_deg) && isNum(f?.speed_mps) && f.speed_mps * 3.6 >= 3) {
      const a = (f.course_deg - 90) * Math.PI / 180;
      ctx.strokeStyle = C.amber; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(cx, cy); ctx.lineTo(cx + Math.cos(a) * 26, cy + Math.sin(a) * 26); ctx.stroke();
    }
    if (f?.valid && isNum(f.accuracy_m)) ring(ctx, cx, cy, Math.max(4, f.accuracy_m * s), "rgba(255,180,60,0.45)");
    glowDot(ctx, cx, cy, 5, C.display);
    const bar = step * s, bx = 14, by = h - 14;
    ctx.strokeStyle = C.text; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(bx, by); ctx.lineTo(bx + bar, by); ctx.stroke();
    label(ctx, step >= 1000 ? `${step / 1000} km` : `${step} m`, bx + bar / 2, by - 10, C.text, 11);
    label(ctx, "N ↑", w - 22, 16, C.muted, 11);
    setText("trail-note", `${pts.length} positions · grid ${step >= 1000 ? `${step / 1000} km` : `${step} m`}`);
  }

  // ── Satellites ────────────────────────────────────────────────────────
  const snrZone = (s) => (s >= 40 ? "" : s >= 25 ? "a" : "r");
  const snrColor = (s) => (s == null ? C.faint : s >= 40 ? C.green : s >= 25 ? C.amber : C.red);
  function renderSky() {
    const sky = S.sky, sats = sky?.satellites || [];
    setText("sat-count", sky ? `${sky.used} used of ${sky.in_view} in view` : "—");
    const dops = [["hdop", sky?.hdop ?? S.fix?.hdop], ["vdop", sky?.vdop ?? S.fix?.vdop], ["pdop", sky?.pdop ?? S.fix?.pdop]];
    for (const [k, v] of dops) {
      setText(`s-${k}`, fmt(v, 2));
      const zone = !isNum(v) ? "" : v <= 2 ? "" : v <= 5 ? "a" : "r";
      meter(`m-${k}`, isNum(v) ? 1 - Math.min(1, Math.max(0, (v - 1) / 9)) : 0, 24, () => zone);
    }
    // legend
    const counts = {};
    for (const s of sats) { const c = (counts[s.constellation] ||= [0, 0]); c[0]++; if (s.used) c[1]++; }
    $("sky-legend").replaceChildren(...Object.entries(counts).map(([k, [n, u]]) => {
      const [, color, name] = SYS[k] || SYS.unknown, sp = el("span");
      const d = el("i", "dot"); d.style.background = color;
      sp.append(d, document.createTextNode(`${name} ${u}/${n}`));
      return sp;
    }));
    // table
    const rows = sats.map((s) => {
      const [letter, color, name] = SYS[s.constellation] || SYS.unknown;
      const tr = el("tr", s.used ? null : "unused");
      const id = el("td", "id", `${letter}${String(s.prn).padStart(2, "0")}`); id.style.color = color;
      const sig = el("div", "sig"), lit = Math.round(Math.min(50, s.snr_dbhz || 0) / 5);
      for (let i = 0; i < 10; i++) sig.append(el("i", i < lit ? `on ${snrZone(s.snr_dbhz)}` : null));
      const sigTd = el("td"); sigTd.append(sig);
      tr.append(id, el("td", null, name), el("td", "n", s.elevation_deg != null ? `${s.elevation_deg}°` : "—"),
        el("td", "n", s.azimuth_deg != null ? `${s.azimuth_deg}°` : "—"), el("td", "n", s.snr_dbhz != null ? String(s.snr_dbhz) : "—"),
        sigTd, el("td", null, s.used ? "●" : "○"));
      return tr;
    });
    $("sat-table").tBodies[0].replaceChildren(...rows);
    drawSky(sats); drawSnr(sats);
  }

  function drawSky(sats) {
    const c = canvas("c-sky"); if (!c) return;
    const { ctx, w, h } = c, cx = w / 2, cy = h / 2, R = Math.min(w, h) / 2 - 22;
    for (const [el_, txt] of [[0, "0°"], [30, "30°"], [60, "60°"]]) {
      const r = R * (1 - el_ / 90);
      ring(ctx, cx, cy, r, el_ === 0 ? C.gridHi : C.grid);
      label(ctx, txt, cx + 4, cy - r + 9, C.faint, 10, "left");
    }
    ctx.strokeStyle = C.grid; ctx.beginPath();
    ctx.moveTo(cx - R, cy); ctx.lineTo(cx + R, cy); ctx.moveTo(cx, cy - R); ctx.lineTo(cx, cy + R); ctx.stroke();
    for (const [t, d] of [["N", 0], ["E", 90], ["S", 180], ["W", 270]]) {
      const a = (d - 90) * Math.PI / 180;
      label(ctx, t, cx + Math.cos(a) * (R + 12), cy + Math.sin(a) * (R + 12), t === "N" ? C.text : C.muted, 12);
    }
    let unplaced = 0;
    for (const s of sats) {
      if (s.elevation_deg == null || s.azimuth_deg == null) { unplaced++; continue; }
      const [letter, color] = SYS[s.constellation] || SYS.unknown;
      const r = R * (1 - Math.max(0, Math.min(90, s.elevation_deg)) / 90), a = (s.azimuth_deg - 90) * Math.PI / 180;
      const x = cx + Math.cos(a) * r, y = cy + Math.sin(a) * r, size = 3 + Math.min(50, s.snr_dbhz || 0) / 10;
      glowDot(ctx, x, y, size, color, s.used);
      label(ctx, `${letter}${String(s.prn).padStart(2, "0")}`, x + size + 3, y - size, s.used ? color : C.muted, 10, "left");
    }
    if (!sats.length) label(ctx, "no satellites reported", cx, cy + 24, C.faint, 13);
    else if (unplaced) label(ctx, `${unplaced} without position`, cx, h - 10, C.faint, 10);
  }

  function drawSnr(sats) {
    const c = canvas("c-snr"); if (!c) return;
    const { ctx, w, h } = c, top = 12, bottom = h - 30, left = 30, right = w - 8, max = 55;
    const y = (v) => bottom - (Math.min(max, v) / max) * (bottom - top);
    ctx.strokeStyle = C.grid; ctx.lineWidth = 1; ctx.beginPath();
    for (const v of [10, 20, 30, 40, 50]) { ctx.moveTo(left, y(v)); ctx.lineTo(right, y(v)); label(ctx, String(v), left - 6, y(v), C.faint, 10, "right"); }
    ctx.stroke();
    for (const [v, col] of [[25, "rgba(255,180,60,0.35)"], [40, "rgba(69,245,161,0.35)"]]) {
      ctx.setLineDash([4, 4]); ctx.strokeStyle = col; ctx.beginPath(); ctx.moveTo(left, y(v)); ctx.lineTo(right, y(v)); ctx.stroke(); ctx.setLineDash([]);
    }
    if (!sats.length) { label(ctx, "no signal data", w / 2, h / 2, C.faint, 13); return; }
    const n = sats.length, slot = (right - left) / n, bw = Math.max(3, Math.min(22, slot * 0.7));
    sats.forEach((s, i) => {
      const x = left + slot * i + (slot - bw) / 2, v = s.snr_dbhz;
      const [letter, color] = SYS[s.constellation] || SYS.unknown;
      if (v != null) {
        ctx.save(); ctx.globalAlpha = s.used ? 1 : 0.45; ctx.fillStyle = snrColor(v);
        if (s.used) { ctx.shadowColor = snrColor(v); ctx.shadowBlur = 6; }
        ctx.fillRect(x, y(v), bw, bottom - y(v)); ctx.restore();
      } else {
        ctx.strokeStyle = C.faint; ctx.strokeRect(x + 0.5, bottom - 3.5, bw - 1, 3);
      }
      if (slot >= 16) {
        ctx.save(); ctx.translate(x + bw / 2, bottom + 6); ctx.rotate(slot < 26 ? -Math.PI / 2.6 : 0);
        label(ctx, `${letter}${String(s.prn).padStart(2, "0")}`, 0, 8, s.used ? color : C.muted, 10, slot < 26 ? "right" : "center");
        ctx.restore();
      }
    });
  }

  // ── System ────────────────────────────────────────────────────────────
  const COUNTERS = [
    ["bytes", "Bytes received", (v) => `${(v / 1024).toFixed(1)} KiB`, (r) => `${(r / 1024).toFixed(2)} KiB/s`],
    ["sentences", "NMEA sentences", String, (r) => `${r.toFixed(1)}/s`],
    ["checksum_errors", "Checksum errors", String, (r) => `${r.toFixed(2)}/s`, "warn"],
    ["malformed", "Malformed lines", String, (r) => `${r.toFixed(2)}/s`, "warn"],
    ["fixes", "Fixes", String, (r) => `${r.toFixed(1)}/s`],
    ["mqtt_published", "MQTT published", String, (r) => `${r.toFixed(1)}/s`],
    ["mqtt_dropped", "MQTT dropped", String, (r) => `${r.toFixed(2)}/s`, "warn"],
    ["archive_errors", "Archive errors", String, (r) => `${r.toFixed(2)}/s`, "bad"],
    ["clients", "TCP clients", String, null],
  ];
  function renderSystem() {
    const st = S.status;
    if (!st) { kv("sys-links", [["Status", "waiting for the API…"]]); return; }
    const h = st.health, linkCls = (s) => (s === "connected" ? "ok" : s === "disabled" ? null : s === "connecting" ? "warn" : "bad");
    kv("sys-links", [
      ["Source", h.source, linkCls(h.source), h.source_detail],
      ["Receiver", st.info.receiver],
      ["MQTT", h.mqtt, linkCls(h.mqtt), st.info.mqtt || ""],
      ["Dashboard", S.streamUp ? "live" : "reconnecting", S.streamUp ? "ok" : "warn", "SSE"],
      ["Sentences", `${fmt(h.sentence_rate_hz, 0)}/s`],
      ["Fix rate", `${fmt(h.fix_rate_hz, 0)} Hz`],
      ["Fix age", isNum(h.fix_age_s) ? `${h.fix_age_s.toFixed(1)} s` : "—", isNum(h.fix_age_s) && h.fix_age_s > 3 ? "warn" : null],
      ["Version", st.info.version],
    ]);
    kv("sys-counters", COUNTERS.map(([k, name, f, rate, cls]) => {
      const v = st.counters[k] ?? 0, r = S.rates[k];
      return [name, f(v), v > 0 && cls ? cls : null, rate && isNum(r) ? rate(r) : ""];
    }));
    const msgs = (st.messages || []).slice().reverse();
    $("sys-messages").replaceChildren(...(msgs.length ? msgs.map((m) => el("li", null, m)) : [el("li", "empty", "none received")]));
    setText("foot-uptime", `up ${Math.floor(h.uptime_s / 3600)}h ${String(Math.floor(h.uptime_s / 60) % 60).padStart(2, "0")}m`);
  }
  function renderNmea(text) {
    const q = $("nmea-filter").value.trim().toUpperCase();
    let lines = text.split(/\r?\n/).filter(Boolean);
    if (q) lines = lines.filter((l) => l.toUpperCase().includes(q));
    lines = lines.slice(-40);
    const pre = $("sys-nmea"), atBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 4;
    pre.replaceChildren(...lines.flatMap((l) => {
      const kind = l.startsWith("$P") ? "P" : l.slice(3, 6);
      return [el("span", /^[A-Z]{3}$/.test(kind) ? kind : null, l), document.createTextNode("\n")];
    }));
    if (atBottom) pre.scrollTop = pre.scrollHeight;
    setText("nmea-note", `${lines.length} ${q ? `matching “${q}”` : "most recent"}`);
  }

  // ── Configuration ─────────────────────────────────────────────────────
  const GROUPS = [
    ["Source", /^(connection_type|port_name|baud_rate|tcp_|udp_|replay_|nmea_require)/],
    ["Receiver", /^(receiver|update_rate_hz|set_gps_to_10hz|init_commands)/],
    ["MQTT", /^mqtt_/],
    ["HTTP", /^http_/],
    ["Network outputs", /^(nmea_tcp|nmea_udp|gpsd_|max_clients)/],
    ["Processing", /^(telemetry_|satellite_)/],
    ["Logging & terminal", /^(log_|tui_|max_log)/],
  ];
  let configRendered = false;
  function renderConfig() {
    const st = S.status;
    if (st) {
      kv("cfg-wiring", [
        ["Source", st.info.source], ["Receiver", st.info.receiver], ["MQTT", st.info.mqtt || "disabled"],
        ...st.info.outputs.map((o, i) => [i ? "" : "Outputs", o]),
      ]);
    }
    const eps = [["health", "open"], ["metrics", "open, Prometheus"], ["api/v1/status", "token"], ["api/v1/fix", "token"],
      ["api/v1/sky", "token"], ["api/v1/geojson", "token"], ["api/v1/nmea", "token"], ["api/v1/stream", "token, SSE"]];
    if (!$("cfg-endpoints").children.length) {
      $("cfg-endpoints").replaceChildren(...eps.map(([p, note]) => {
        const li = el("li"), a = el("a", null, `/${p}`); a.href = p; a.target = "_blank"; a.rel = "noopener";
        li.append(a, el("small", null, note)); return li;
      }));
    }
    if (!S.config || configRendered) return;
    configRendered = true;
    const used = new Set(), sections = [];
    for (const [title, re] of GROUPS) {
      const keys = Object.keys(S.config).filter((k) => re.test(k) && !used.has(k)).sort();
      keys.forEach((k) => used.add(k));
      sections.push([title, keys]);
    }
    const rest = Object.keys(S.config).filter((k) => !used.has(k) && k !== "config_path");
    if (rest.length) sections.push(["Other", rest]);
    $("cfg-groups").replaceChildren(...sections.filter(([, k]) => k.length).map(([title, keys]) => {
      const sec = el("section"), dl = el("dl", "kv");
      sec.append(el("h3", null, title), dl);
      for (const k of keys) {
        const v = S.config[k];
        const text = v == null ? "unset" : Array.isArray(v) ? (v.length ? v.join(", ") : "[]") : String(v);
        dl.append(el("dt", null, k), el("dd", v == null ? "unset" : null, text));
      }
      return sec;
    }));
  }

  // ── controls ──────────────────────────────────────────────────────────
  function setMode(m) {
    S.mode = m; store.set("mode", m);
    if (location.hash !== `#${m}`) history.replaceState(null, "", `#${m}`);
    for (const id of MODES) $(`mode-${id}`).hidden = id !== m;
    document.querySelectorAll(".key[data-mode]").forEach((k) => k.classList.toggle("active", k.dataset.mode === m));
    if (m === "system") pollNmea();
    schedule();
  }
  document.querySelectorAll(".key[data-mode]").forEach((k) => k.addEventListener("click", () => setMode(k.dataset.mode)));
  window.addEventListener("hashchange", () => { const m = location.hash.slice(1); if (MODES.includes(m)) setMode(m); });
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, dialog")) return;
    const i = "12345".indexOf(e.key);
    if (i >= 0) setMode(MODES[i]);
  });

  const order = Object.keys(UNITS);
  function applyUnits() { setText("k-units-t", U().label); schedule(); }
  $("k-units").addEventListener("click", () => {
    S.units = order[(order.indexOf(S.units) + 1) % order.length]; store.set("units", S.units); applyUnits();
  });
  function applyDim() { document.body.classList.toggle("dim", S.dim); setText("k-dim-t", S.dim ? "Dimmed" : "Bright"); }
  $("k-dim").addEventListener("click", () => { S.dim = !S.dim; store.set("dim", S.dim ? "1" : "0"); applyDim(); });

  $("k-copy").addEventListener("click", async () => {
    const f = S.fix;
    if (!f?.valid) return;
    try { await navigator.clipboard.writeText(`${f.latitude.toFixed(7)}, ${f.longitude.toFixed(7)}`); flash("k-copy"); } catch { /* insecure context */ }
  });
  $("k-copy-mgrs").addEventListener("click", async () => {
    const f = S.fix;
    if (!f?.valid || !f.mgrs) return;
    try { await navigator.clipboard.writeText(f.mgrs); flash("k-copy-mgrs"); } catch { /* insecure context */ }
  });
  $("k-geojson").addEventListener("click", async (e) => {
    e.preventDefault();
    try {
      const r = await api("api/v1/geojson");
      if (!r.ok) return;
      const url = URL.createObjectURL(await r.blob()), a = el("a");
      a.href = url; a.download = "position.geojson"; a.click();
      setTimeout(() => URL.revokeObjectURL(url), 5000);
    } catch { /* unauthorized handled */ }
  });
  $("k-trail-clear").addEventListener("click", () => { S.trail = []; S.gg = []; S.peak = 0; schedule(); });
  $("k-view-map").addEventListener("click", () => { S.view = "map"; store.set("trackView", "map"); schedule(); });
  $("k-view-plot").addEventListener("click", () => { S.view = "plot"; store.set("trackView", "plot"); schedule(); });
  $("k-zoom-in").addEventListener("click", () => setZoom(S.zoom + 1));
  $("k-zoom-out").addEventListener("click", () => setZoom(S.zoom - 1));
  $("k-follow").addEventListener("click", () => { S.follow = !S.follow; schedule(); });
  {
    // Drag to pan (stops following), wheel to zoom.
    const box = $("mapbox");
    let drag = null;
    box.addEventListener("pointerdown", (e) => {
      if (!(mapEnabled() && S.view === "map" && S.center)) return;
      drag = { x: e.clientX, y: e.clientY, c: project(S.center[0], S.center[1], S.zoom) };
      box.setPointerCapture(e.pointerId); box.classList.add("dragging");
    });
    box.addEventListener("pointermove", (e) => {
      if (!drag) return;
      S.follow = false;
      S.center = unproject(drag.c[0] - (e.clientX - drag.x), drag.c[1] - (e.clientY - drag.y), S.zoom);
      schedule();
    });
    const end = () => { drag = null; box.classList.remove("dragging"); };
    box.addEventListener("pointerup", end);
    box.addEventListener("pointercancel", end);
    box.addEventListener("wheel", (e) => {
      // Ctrl/⌘ + wheel (and trackpad pinch) zooms; a plain wheel keeps scrolling the page.
      if (!(mapEnabled() && S.view === "map") || !(e.ctrlKey || e.metaKey)) return;
      e.preventDefault(); setZoom(S.zoom + (e.deltaY < 0 ? 1 : -1));
    }, { passive: false });
  }
  $("k-nmea-pause").addEventListener("click", () => {
    S.nmeaPaused = !S.nmeaPaused;
    $("k-nmea-pause").classList.toggle("active", S.nmeaPaused);
    $("k-nmea-pause").querySelector("span").textContent = S.nmeaPaused ? "Resume" : "Pause";
    if (!S.nmeaPaused) pollNmea();
  });
  $("nmea-filter").addEventListener("input", pollNmea);
  function flash(id) { const k = $(id); k.classList.add("active"); setTimeout(() => k.classList.remove("active"), 600); }

  new ResizeObserver(schedule).observe(document.body);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) { pollStatus(); schedule(); } });

  // ── start ─────────────────────────────────────────────────────────────
  applyUnits(); applyDim(); setMode(S.mode);
  loadOnce(); pollStatus(); stream();
  setInterval(pollStatus, 2000);
  setInterval(pollNmea, 1000);
  setInterval(schedule, 1000); // ages, stale states, fading trails
})();
