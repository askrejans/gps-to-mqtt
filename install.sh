#!/bin/bash
set -e

# GPS to MQTT Installation Script
# This script installs the gps-to-mqtt service on a Linux system

INSTALL_DIR="/opt/gps-to-mqtt"
CONFIG_DIR="/etc/g86-car-telemetry"
LOG_DIR="/var/log/gps-to-mqtt"
SERVICE_USER="gps"
SERVICE_GROUP="gps"

echo "=== GPS to MQTT Installation Script ==="
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

# Check if binary exists
if [ ! -f "target/release/gps-to-mqtt" ]; then
    echo "ERROR: Binary not found. Please build the project first:"
    echo "  cargo build --release"
    exit 1
fi

echo "Step 1: Creating directories..."
mkdir -p "$INSTALL_DIR"
mkdir -p "$CONFIG_DIR"
mkdir -p "$LOG_DIR"

echo "Step 2: Creating service user and group..."
if ! id "$SERVICE_USER" &>/dev/null; then
    useradd -r -s /bin/false -d "$INSTALL_DIR" "$SERVICE_USER"
    echo "  Created user: $SERVICE_USER"
else
    echo "  User $SERVICE_USER already exists"
fi

echo "Step 3: Installing binary..."
cp target/release/gps-to-mqtt "$INSTALL_DIR/"
chmod 755 "$INSTALL_DIR/gps-to-mqtt"
chown root:root "$INSTALL_DIR/gps-to-mqtt"
echo "  Installed to $INSTALL_DIR/gps-to-mqtt"

echo "Step 4: Installing configuration..."
if [ ! -f "$CONFIG_DIR/gps-to-mqtt.toml" ]; then
    cp example.settings.toml "$CONFIG_DIR/gps-to-mqtt.toml"
    echo "  Installed config to $CONFIG_DIR/gps-to-mqtt.toml"
    echo "  WARNING: Please edit the configuration file before starting the service!"
else
    echo "  Config file already exists, not overwriting"
fi

echo "Step 5: Setting permissions..."
chown -R "$SERVICE_USER:$SERVICE_GROUP" "$LOG_DIR"
chmod 755 "$CONFIG_DIR"
chmod 644 "$CONFIG_DIR/gps-to-mqtt.toml"

# Add service user to dialout group for serial port access
if getent group dialout > /dev/null 2>&1; then
    usermod -a -G dialout "$SERVICE_USER"
    echo "  Added $SERVICE_USER to dialout group"
fi

echo "Step 6: Installing systemd service..."
cp gps-to-mqtt.service /etc/systemd/system/
chmod 644 /etc/systemd/system/gps-to-mqtt.service
systemctl daemon-reload
echo "  Service file installed"

echo ""
echo "=== Installation Complete ==="
echo ""
echo "Next steps:"
echo "  1. Edit the configuration file:"
echo "     sudo nano $CONFIG_DIR/gps-to-mqtt.toml"
echo ""
echo "  2. Enable the service to start on boot:"
echo "     sudo systemctl enable gps-to-mqtt"
echo ""
echo "  3. Start the service:"
echo "     sudo systemctl start gps-to-mqtt"
echo ""
echo "  4. Check service status:"
echo "     sudo systemctl status gps-to-mqtt"
echo ""
echo "  5. View logs:"
echo "     sudo journalctl -u gps-to-mqtt -f"
echo ""
