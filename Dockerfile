# ── Build stage ────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

# cmake + perl for aws-lc-sys (rustls), pkg-config + libudev-dev for serialport
RUN apt-get update && apt-get install -y --no-install-recommends \
        perl make cmake pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies: build once against a stub main, then the real sources
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --locked 2>/dev/null || true \
    && rm -rf src

COPY . .
RUN touch src/main.rs && cargo build --release --locked

# ── Runtime stage ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r gps \
    && useradd -r -g gps --groups dialout --no-create-home --shell /usr/sbin/nologin gps \
    && mkdir -p /etc/gps-to-mqtt /var/lib/gps-to-mqtt \
    && chown gps:gps /var/lib/gps-to-mqtt

COPY --from=builder /app/target/release/gps-to-mqtt /usr/local/bin/gps-to-mqtt
COPY example.settings.toml /etc/gps-to-mqtt/settings.toml
COPY LICENSE /usr/share/doc/gps-to-mqtt/copyright

USER gps
WORKDIR /var/lib/gps-to-mqtt

# HTTP API on by default so the container health probe works; loopback only
# unless GPS_TO_MQTT_HTTP_BIND=0.0.0.0 (then also set GPS_TO_MQTT_HTTP_TOKEN).
ENV GPS_TO_MQTT_HTTP_ENABLED=true \
    GPS_TO_MQTT_HTTP_BIND=127.0.0.1

# HTTP API / metrics, NMEA-0183 TCP, gpsd
EXPOSE 9090 10110 2947

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["gps-to-mqtt", "--healthcheck"]

# Configure with GPS_TO_MQTT_* environment variables or mount a settings.toml.
ENTRYPOINT ["gps-to-mqtt", "--headless"]
CMD ["--config", "/etc/gps-to-mqtt/settings.toml"]
