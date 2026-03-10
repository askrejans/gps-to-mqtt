# ── Build stage ────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

# Build dependencies: cmake + perl for aws-lc-sys (TLS), pkg-config for linkage
RUN apt-get update && apt-get install -y --no-install-recommends \
        perl \
        make \
        cmake \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies: copy manifests first, build a dummy main, then replace
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --locked 2>/dev/null || true \
    && rm -rf src

# Build the real binary
COPY . .
RUN cargo build --release --locked

# ── Runtime stage ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r gps \
    && useradd -r -g gps \
        --groups dialout \
        --no-create-home \
        --shell /usr/sbin/nologin \
        gps \
    && mkdir -p /etc/gps-to-mqtt

COPY --from=builder /app/target/release/gps-to-mqtt /usr/local/bin/gps-to-mqtt
COPY example.settings.toml /etc/gps-to-mqtt/settings.toml

USER gps
WORKDIR /tmp

# All configuration is driven by environment variables (GPS_TO_MQTT_* prefix)
# or by mounting a settings.toml over /etc/gps-to-mqtt/settings.toml.
CMD ["gps-to-mqtt", "--config", "/etc/gps-to-mqtt/settings.toml"]
