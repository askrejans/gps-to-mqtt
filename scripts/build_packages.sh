#!/usr/bin/env bash
# build_packages.sh — Build DEB and/or RPM packages for gps-to-mqtt.
#
# Requirements:
#   cargo, cross (for cross-compilation), fpm (for packaging)
#
# Usage:
#   ./scripts/build_packages.sh [--arch x86-64|arm64|all] [--type deb|rpm|all] [--no-cross] [--help]

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
ARCH="all"
PKG_TYPE="all"
USE_CROSS=true
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Options:
  --arch <x86-64|arm64|all>    Target architecture (default: all)
  --type <deb|rpm|all>         Package format (default: all)
  --no-cross                   Use native cargo instead of cross
  -h, --help                   Show this help

Examples:
  $0
  $0 --arch arm64 --type deb
  $0 --arch x86-64 --type rpm --no-cross
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)   ARCH="$2";     shift 2 ;;
        --type)   PKG_TYPE="$2"; shift 2 ;;
        --no-cross) USE_CROSS=false; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Resolve package metadata from Cargo.toml
# ---------------------------------------------------------------------------
PKG_NAME=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0]['name'])")
PKG_VERSION=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0]['version'])")
PKG_DESC=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0].get('description',''))")
PKG_LICENSE=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0].get('license','MIT'))")

echo "┌──────────────────────────────────────────────────"
echo "│  Building packages for ${PKG_NAME} v${PKG_VERSION}"
echo "│  Arch: ${ARCH}  |  Type: ${PKG_TYPE}  |  Cross: ${USE_CROSS}"
echo "└──────────────────────────────────────────────────"

# ---------------------------------------------------------------------------
# Select architectures
# ---------------------------------------------------------------------------
declare -a ARCHS=()
case "$ARCH" in
    x86-64) ARCHS=("x86-64") ;;
    arm64)  ARCHS=("arm64")  ;;
    all)    ARCHS=("x86-64" "arm64") ;;
    *) echo "Unknown arch: $ARCH"; exit 1 ;;
esac

# ---------------------------------------------------------------------------
# Select package types
# ---------------------------------------------------------------------------
declare -a PKG_TYPES=()
case "$PKG_TYPE" in
    deb) PKG_TYPES=("deb") ;;
    rpm) PKG_TYPES=("rpm") ;;
    all) PKG_TYPES=("deb" "rpm") ;;
    *) echo "Unknown type: $PKG_TYPE"; exit 1 ;;
esac

# ---------------------------------------------------------------------------
# Check for required tools
# ---------------------------------------------------------------------------
check_tool() {
    command -v "$1" >/dev/null 2>&1 || { echo "ERROR: '$1' not found. $2"; exit 1; }
}
check_tool cargo "Install Rust: https://rustup.rs"
check_tool fpm   "Install fpm: gem install fpm"
if [[ "$USE_CROSS" == "true" ]]; then
    check_tool cross "Install cross: cargo install cross"
fi

# ---------------------------------------------------------------------------
# Helper: resolve Rust triple and fpm arch string
# ---------------------------------------------------------------------------
arch_to_triple() {
    case "$1" in
        x86-64) echo "x86_64-unknown-linux-musl" ;;
        arm64)  echo "aarch64-unknown-linux-musl" ;;
    esac
}

arch_to_deb_arch() {
    case "$1" in
        x86-64) echo "amd64" ;;
        arm64)  echo "arm64" ;;
    esac
}

arch_to_rpm_arch() {
    case "$1" in
        x86-64) echo "x86_64" ;;
        arm64)  echo "aarch64" ;;
    esac
}

# ---------------------------------------------------------------------------
# Build loop
# ---------------------------------------------------------------------------
DIST_DIR="${REPO_ROOT}/dist"
mkdir -p "${DIST_DIR}"

for arch in "${ARCHS[@]}"; do
    triple=$(arch_to_triple "$arch")
    echo
    echo "==> Compiling for ${arch} (${triple})"

    cd "${REPO_ROOT}"

    if [[ "$USE_CROSS" == "true" ]]; then
        cross build --release --target "${triple}"
    else
        cargo build --release --target "${triple}"
    fi

    BINARY="${REPO_ROOT}/target/${triple}/release/${PKG_NAME}"

    if [[ ! -f "$BINARY" ]]; then
        echo "ERROR: binary not found at ${BINARY}"
        exit 1
    fi

    # -------------------------------------------------------------------------
    # Staging area
    # -------------------------------------------------------------------------
    STAGE=$(mktemp -d)
    trap 'rm -rf "${STAGE}"' EXIT

    install -Dm755 "${BINARY}"                  "${STAGE}/usr/bin/${PKG_NAME}"
    install -Dm644 "${REPO_ROOT}/gps-to-mqtt.service" \
                                                 "${STAGE}/lib/systemd/system/${PKG_NAME}.service"
    install -Dm644 "${REPO_ROOT}/example.settings.toml" \
                                                 "${STAGE}/etc/${PKG_NAME}/settings.toml.example"

    # -------------------------------------------------------------------------
    # Package
    # -------------------------------------------------------------------------
    for pkg_type in "${PKG_TYPES[@]}"; do
        if [[ "$pkg_type" == "deb" ]]; then
            fpm_arch=$(arch_to_deb_arch "$arch")
        else
            fpm_arch=$(arch_to_rpm_arch "$arch")
        fi

        OUTPUT="${DIST_DIR}/${PKG_NAME}_${PKG_VERSION}_${fpm_arch}.${pkg_type}"

        echo "  --> Creating ${pkg_type} package: $(basename "${OUTPUT}")"

        fpm \
            --input-type dir \
            --output-type "${pkg_type}" \
            --name "${PKG_NAME}" \
            --version "${PKG_VERSION}" \
            --license "${PKG_LICENSE}" \
            --description "${PKG_DESC}" \
            --architecture "${fpm_arch}" \
            --package "${OUTPUT}" \
            --after-install /dev/stdin \
            --chdir "${STAGE}" \
            . <<'POSTINSTALL'
#!/bin/sh
systemctl daemon-reload || true
if ! id gps >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin gps
fi
POSTINSTALL

        echo "     Saved: ${OUTPUT}"
    done

    rm -rf "${STAGE}"
    trap - EXIT
done

echo
echo "All packages written to: ${DIST_DIR}/"
ls -lh "${DIST_DIR}/"
