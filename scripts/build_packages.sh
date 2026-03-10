#!/usr/bin/env bash
# =============================================================================
# build_packages.sh – Cross-compile gps-to-mqtt for all platforms
#
# Usage:
#   ./scripts/build_packages.sh [options]
#
# Options:
#   --platform  linux|windows|mac|all   Target OS (default: all)
#   --arch      x86|x64|arm|arm64|all  CPU architecture (default: all)
#   --type      deb|rpm|zip|targz|all  Package format (default: all)
#   --no-cross  Use local cargo instead of 'cross' for Linux/Windows targets
#   --help      Show this help
#
# Cross-compilation targets produced:
#   Linux   x64    → x86_64-unknown-linux-gnu          → .deb + .rpm
#   Linux   arm64  → aarch64-unknown-linux-gnu          → .deb + .rpm
#   Windows x64    → x86_64-pc-windows-gnu              → .zip
#   macOS   x64    → x86_64-apple-darwin                → .tar.gz
#   macOS   arm64  → aarch64-apple-darwin               → .tar.gz
#
# Mac prerequisites (brew install + rustup):
#   Linux/Win:  cargo install cross  +  Docker (for cross)
#   macOS:      rustup target add x86_64-apple-darwin aarch64-apple-darwin
#               Xcode Command Line Tools (xcode-select --install)
#
# Outputs are written to:
#   release/<version>/linux/deb/
#   release/<version>/linux/rpm/
#   release/<version>/windows/
#   release/<version>/mac/
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
PLATFORM_TARGET="all"
ARCH_TARGET="all"
PKG_TYPE="all"
USE_CROSS=true
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --platform) PLATFORM_TARGET="$2"; shift 2 ;;
        --arch)     ARCH_TARGET="$2";     shift 2 ;;
        --type)     PKG_TYPE="$2";        shift 2 ;;
        --no-cross) USE_CROSS=false;      shift   ;;
        --help)
            head -30 "$0" | grep "^#" | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log()  { echo "[BUILD] $*" >&2; }
warn() { echo "[WARN]  $*" >&2; }
die()  { echo "[ERROR] $*" >&2; exit 1; }

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' is not installed. $2"
}

# ---------------------------------------------------------------------------
# Package metadata
# ---------------------------------------------------------------------------
PKG_NAME="gps-to-mqtt"
PKG_VERSION="$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"
PKG_DESCRIPTION="GPS to MQTT bridge service"
PKG_MAINTAINER="askrejans <arvis.skrejans@gmail.com>"
PKG_LICENSE="MIT"
PKG_URL="https://github.com/askrejans/gps-to-mqtt"
SERVICE_FILE="$PROJECT_DIR/gps-to-mqtt.service"

log "Package: $PKG_NAME v$PKG_VERSION"

RELEASE_DIR="$PROJECT_DIR/release/$PKG_VERSION"
LINUX_DEB_DIR="$RELEASE_DIR/linux/deb"
LINUX_RPM_DIR="$RELEASE_DIR/linux/rpm"
WIN_DIR="$RELEASE_DIR/windows"
MAC_DIR="$RELEASE_DIR/mac"

# ---------------------------------------------------------------------------
# Target triple lookup
# ---------------------------------------------------------------------------
get_triple() {
    case "$1" in
        linux-x64)   echo "x86_64-unknown-linux-gnu" ;;
        linux-arm64) echo "aarch64-unknown-linux-gnu" ;;
        win-x64)     echo "x86_64-pc-windows-gnu" ;;
        mac-x64)     echo "x86_64-apple-darwin" ;;
        mac-arm64)   echo "aarch64-apple-darwin" ;;
        *) die "Unknown target '$1'" ;;
    esac
}

# Human-readable package arch strings used in filenames / control files
deb_arch_of() {
    case "$1" in
        linux-x64)   echo "amd64" ;;
        linux-arm64) echo "arm64" ;;
    esac
}

rpm_arch_of() {
    case "$1" in
        linux-x64)   echo "x86_64" ;;
        linux-arm64) echo "aarch64" ;;
    esac
}

# ---------------------------------------------------------------------------
# Build active targets list
# ---------------------------------------------------------------------------
LINUX_TARGETS=()
WIN_TARGETS=()
MAC_TARGETS=()

add_linux_arch() {
    case "$1" in
        x64)   LINUX_TARGETS+=("linux-x64") ;;
        arm64) LINUX_TARGETS+=("linux-arm64") ;;
        all)   LINUX_TARGETS+=("linux-x64" "linux-arm64") ;;
        x86|arm) warn "32-bit Linux targets are disabled; skipping" ;;
        *) die "Unknown arch '$1' for linux" ;;
    esac
}

add_win_arch() {
    case "$1" in
        x64)   WIN_TARGETS+=("win-x64") ;;
        all)   WIN_TARGETS+=("win-x64") ;;
        x86)   warn "32-bit Windows target is disabled; skipping" ;;
        arm|arm64)
            warn "Windows arm/arm64 cross-compilation is not supported; skipping" ;;
        *) die "Unknown arch '$1' for windows" ;;
    esac
}

add_mac_arch() {
    case "$1" in
        x64)   MAC_TARGETS+=("mac-x64") ;;
        arm64) MAC_TARGETS+=("mac-arm64") ;;
        all)   MAC_TARGETS+=("mac-x64" "mac-arm64") ;;
        x86|arm)
            warn "macOS $1 is not a supported target; skipping" ;;
        *) die "Unknown arch '$1' for mac" ;;
    esac
}

case "$PLATFORM_TARGET" in
    linux)
        add_linux_arch "$ARCH_TARGET"
        ;;
    windows)
        add_win_arch "$ARCH_TARGET"
        ;;
    mac)
        add_mac_arch "$ARCH_TARGET"
        ;;
    all)
        add_linux_arch "$ARCH_TARGET"
        add_win_arch   "$ARCH_TARGET"
        add_mac_arch   "$ARCH_TARGET"
        ;;
    *) die "Unknown platform '$PLATFORM_TARGET'. Use linux, windows, mac, or all." ;;
esac

# ---------------------------------------------------------------------------
# Docker check – must be running for Linux/Windows cross-compilation via cross
# ---------------------------------------------------------------------------
check_docker() {
    if $USE_CROSS; then
        require_cmd docker "Install Docker Desktop from https://docs.docker.com/desktop/mac/"
        if ! docker info >/dev/null 2>&1; then
            die "Docker is not running. Start Docker Desktop, wait for it to be ready, then retry."
        fi
        log "Docker is running."
        # cross Docker images are x86_64-only. On Apple Silicon, tell Docker to
        # pull and run the amd64 image via Rosetta emulation (transparent, fast).
        export DOCKER_DEFAULT_PLATFORM=linux/amd64
    fi
}

# ---------------------------------------------------------------------------
# Compile binary
# Returns path to the compiled binary.
# For macOS targets, always uses native cargo (cross doesn't support darwin).
# For Linux/Windows, uses 'cross' unless --no-cross.
# ---------------------------------------------------------------------------
build_binary() {
    local target="$1"   # e.g. linux-x64
    local triple
    triple="$(get_triple "$target")"
    local bin_ext=""
    [[ "$target" == win-* ]] && bin_ext=".exe"
    local bin_dir="$PROJECT_DIR/target/$triple/release"
    local bin_out="$bin_dir/${PKG_NAME}${bin_ext}"

    log "Compiling for $triple …"

    if [[ "$target" == mac-* ]]; then
        # macOS targets must be built natively on macOS with the Apple SDK.
        rustup target add "$triple" 2>/dev/null || true
        (cd "$PROJECT_DIR" && cargo build --release --target "$triple")
    elif $USE_CROSS; then
        require_cmd cross "Install with: cargo install cross  (also needs Docker)"
        # Run cross from the project dir – cross maps cwd into the container,
        # so --manifest-path with a host path breaks inside Docker.
        # cross container images are amd64-only; on Apple Silicon use Rosetta via DOCKER_DEFAULT_PLATFORM.
        (cd "$PROJECT_DIR" && DOCKER_DEFAULT_PLATFORM=linux/amd64 cross build --release --target "$triple")
    else
        rustup target add "$triple" 2>/dev/null || true
        (cd "$PROJECT_DIR" && cargo build --release --target "$triple")
    fi

    # Verify the binary was actually produced – catches silent build failures.
    [[ -f "$bin_out" ]] || \
        die "Binary not found after build: $bin_out  (did compilation succeed?)"

    echo "$bin_out"
}

# ---------------------------------------------------------------------------
# DEB packaging
# ---------------------------------------------------------------------------
build_deb() {
    local target="$1"   # e.g. linux-x64
    require_cmd dpkg-deb "Install: sudo apt-get install dpkg-dev  OR  brew install dpkg"

    local bin_path
    bin_path="$(build_binary "$target")"
    local deb_arch
    deb_arch="$(deb_arch_of "$target")"

    local pkg_root="$RELEASE_DIR/deb-build-${target}"
    local install_prefix="$pkg_root/usr/bin"
    local service_dir="$pkg_root/lib/systemd/system"
    local config_dir="$pkg_root/etc/$PKG_NAME"
    local debian_dir="$pkg_root/DEBIAN"

    mkdir -p "$install_prefix" "$service_dir" "$config_dir" "$debian_dir"

    cp "$bin_path" "$install_prefix/$PKG_NAME"
    chmod 755 "$install_prefix/$PKG_NAME"

    [[ -f "$SERVICE_FILE" ]] && \
        cp "$SERVICE_FILE" "$service_dir/${PKG_NAME}.service" || \
        warn "Service file not found at $SERVICE_FILE – skipping"

    [[ -f "$PROJECT_DIR/example.settings.toml" ]] && \
        cp "$PROJECT_DIR/example.settings.toml" "$config_dir/settings.toml.example"

    cat > "$debian_dir/control" <<EOF
Package: $PKG_NAME
Version: $PKG_VERSION
Architecture: $deb_arch
Maintainer: $PKG_MAINTAINER
Description: $PKG_DESCRIPTION
 Bridges a GPS device (serial or TCP/IP) to an MQTT broker.
 Runs as a systemd service in production or as an interactive TUI.
Section: misc
Priority: optional
Homepage: $PKG_URL
EOF

    cat > "$debian_dir/postinst" <<'POSTINST'
#!/bin/bash
set -e
# Create service user in dialout+tty groups for serial GPS access
if ! id gps &>/dev/null; then
    useradd --system --no-create-home --shell /usr/sbin/nologin \
            --groups dialout,tty gps 2>/dev/null || true
fi
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable gps-to-mqtt.service || true
    echo "Service installed. Edit /etc/gps-to-mqtt/settings.toml, then:"
    echo "  sudo systemctl start gps-to-mqtt"
fi
exit 0
POSTINST
    chmod 755 "$debian_dir/postinst"

    cat > "$debian_dir/prerm" <<'PRERM'
#!/bin/bash
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop    gps-to-mqtt.service 2>/dev/null || true
    systemctl disable gps-to-mqtt.service 2>/dev/null || true
fi
exit 0
PRERM
    chmod 755 "$debian_dir/prerm"

    mkdir -p "$LINUX_DEB_DIR"
    local deb_file="$LINUX_DEB_DIR/${PKG_NAME}_${PKG_VERSION}_${deb_arch}.deb"
    dpkg-deb --root-owner-group --build "$pkg_root" "$deb_file"
    rm -rf "$pkg_root"
    log "DEB created: $deb_file"
}

# ---------------------------------------------------------------------------
# RPM packaging
# ---------------------------------------------------------------------------
build_rpm() {
    local target="$1"
    require_cmd rpmbuild "Install (Fedora/RHEL): sudo dnf install rpm-build  OR  brew install rpm"

    local bin_path
    bin_path="$(build_binary "$target")"
    local rpm_arch
    rpm_arch="$(rpm_arch_of "$target")"

    local build_root="$RELEASE_DIR/rpm-build-${target}"
    local rpm_sources="$build_root/SOURCES"
    local rpm_specs="$build_root/SPECS"
    mkdir -p "$rpm_sources" "$rpm_specs" \
             "$build_root/BUILD" "$build_root/BUILDROOT" \
             "$build_root/RPMS"  "$build_root/SRPMS"

    cp "$bin_path" "$rpm_sources/$PKG_NAME"
    [[ -f "$SERVICE_FILE" ]] && \
        cp "$SERVICE_FILE" "$rpm_sources/${PKG_NAME}.service"
    [[ -f "$PROJECT_DIR/example.settings.toml" ]] && \
        cp "$PROJECT_DIR/example.settings.toml" "$rpm_sources/settings.toml.example"

    local spec_file="$rpm_specs/${PKG_NAME}.spec"
    cat > "$spec_file" <<EOF
Name:           $PKG_NAME
Version:        $PKG_VERSION
Release:        1%{?dist}
Summary:        $PKG_DESCRIPTION
License:        $PKG_LICENSE
URL:            $PKG_URL
BuildArch:      $rpm_arch

%global debug_package %{nil}

# Fallback systemd macros for building in minimal environments
%{!?systemd_post:                    %define systemd_post(p)                    :}
%{!?systemd_preun:                   %define systemd_preun(p)                   :}
%{!?systemd_postun_with_restart:     %define systemd_postun_with_restart(p)     :}

%description
Bridges a GPS device (serial or TCP/IP) to an MQTT broker.
Runs as a systemd service in production or as an interactive TUI.

%prep

%build

%install
mkdir -p %{buildroot}%{_bindir}
install -m755 %{_sourcedir}/$PKG_NAME %{buildroot}%{_bindir}/$PKG_NAME
mkdir -p %{buildroot}%{_unitdir}
install -m644 %{_sourcedir}/${PKG_NAME}.service %{buildroot}%{_unitdir}/${PKG_NAME}.service
mkdir -p %{buildroot}%{_sysconfdir}/$PKG_NAME
install -m644 %{_sourcedir}/settings.toml.example \
              %{buildroot}%{_sysconfdir}/$PKG_NAME/settings.toml.example

%pre
getent group  dialout >/dev/null || groupadd -r dialout || true
getent passwd gps >/dev/null || \
    useradd --system --no-create-home --shell /sbin/nologin \
            -G dialout,tty gps || true

%files
%{_bindir}/$PKG_NAME
%{_unitdir}/${PKG_NAME}.service
%dir %{_sysconfdir}/$PKG_NAME
%config(noreplace) %{_sysconfdir}/$PKG_NAME/settings.toml.example

%post
%systemd_post ${PKG_NAME}.service
echo "Service installed. Copy and edit the config, then start:"
echo "  sudo cp /etc/$PKG_NAME/settings.toml.example /etc/$PKG_NAME/settings.toml"
echo "  sudo systemctl start $PKG_NAME"

%preun
%systemd_preun ${PKG_NAME}.service

%postun
%systemd_postun_with_restart ${PKG_NAME}.service
EOF

    mkdir -p "$LINUX_RPM_DIR"

    local host_arch
    host_arch="$(uname -m)"
    local host_os
    host_os="$(uname -s)"

    local need_docker=false
    if [[ "$host_os" == "Darwin" ]]; then
        # Always use Docker on macOS: native rpmbuild stamps packages with OS=darwin,
        # causing "intended for a different operating system" errors on Linux RPM systems.
        need_docker=true
    elif [[ ("$host_arch" == "arm64" || "$host_arch" == "aarch64") && "$rpm_arch" == "x86_64" ]]; then
        need_docker=true
    fi

    if $need_docker; then
        local docker_platform
        docker_platform="linux/$([[ "$rpm_arch" == "x86_64" ]] && echo "amd64" || echo "arm64")"
        log "  Building $rpm_arch RPM via Docker ($docker_platform)…"
        docker run --rm \
            --platform "$docker_platform" \
            -v "$build_root:/build_root" \
            fedora:latest \
            bash -c "
                dnf install -yq rpm-build >/dev/null 2>&1
                rpmbuild \
                    --define '_topdir /build_root' \
                    --define '_bindir /usr/bin' \
                    --define '_sbindir /usr/sbin' \
                    --define '_sysconfdir /etc' \
                    --define '_unitdir /usr/lib/systemd/system' \
                    --define 'dist %{nil}' \
                    -bb /build_root/SPECS/${PKG_NAME}.spec
            "
    else
        rpmbuild \
            --define "_topdir $build_root" \
            --define "_bindir /usr/bin" \
            --define "_sbindir /usr/sbin" \
            --define "_sysconfdir /etc" \
            --define "_unitdir /usr/lib/systemd/system" \
            --define "_build_cpu $rpm_arch" \
            --define "_host_cpu $rpm_arch" \
            --define "_target_cpu $rpm_arch" \
            -bb "$spec_file"
    fi

    find "$build_root/RPMS" -name '*.rpm' -exec cp {} "$LINUX_RPM_DIR/" \;
    rm -rf "$build_root"
    log "RPM created in: $LINUX_RPM_DIR"
}

# ---------------------------------------------------------------------------
# Windows ZIP (binary + example config)
# ---------------------------------------------------------------------------
build_win_zip() {
    local target="$1"   # win-x64
    local bin_path
    bin_path="$(build_binary "$target")"

    local arch_label
    case "$target" in
        win-x64) arch_label="windows-x64" ;;
    esac

    mkdir -p "$WIN_DIR"
    local stage_dir="$RELEASE_DIR/win-stage-${target}"
    mkdir -p "$stage_dir"

    cp "$bin_path" "$stage_dir/${PKG_NAME}.exe"
    [[ -f "$PROJECT_DIR/example.settings.toml" ]] && \
        cp "$PROJECT_DIR/example.settings.toml" "$stage_dir/settings.toml.example"
    cat > "$stage_dir/README.txt" <<EOF
gps-to-mqtt v$PKG_VERSION – Windows

Usage:
  gps-to-mqtt.exe --config settings.toml

1. Copy settings.toml.example to settings.toml and edit it.
2. Run the .exe in a terminal or install as a Windows Service with NSSM:
     nssm install gps-to-mqtt "C:\path\to\gps-to-mqtt.exe"
     nssm set    gps-to-mqtt AppParameters "--config C:\path\to\settings.toml"
     nssm start  gps-to-mqtt

Project: $PKG_URL
EOF

    local zip_file="$WIN_DIR/${PKG_NAME}_${PKG_VERSION}_${arch_label}.zip"
    (cd "$stage_dir" && zip -r "$zip_file" .)
    rm -rf "$stage_dir"
    log "Windows ZIP created: $zip_file"
}

# ---------------------------------------------------------------------------
# macOS tar.gz (used by Homebrew and direct download)
# ---------------------------------------------------------------------------
build_mac_targz() {
    local target="$1"   # mac-x64 or mac-arm64
    local bin_path
    bin_path="$(build_binary "$target")"

    local arch_label
    case "$target" in
        mac-x64)   arch_label="macos-x86_64" ;;
        mac-arm64) arch_label="macos-arm64" ;;
    esac

    mkdir -p "$MAC_DIR"
    local stage_dir="$RELEASE_DIR/mac-stage-${target}"
    mkdir -p "$stage_dir"

    cp "$bin_path" "$stage_dir/$PKG_NAME"
    chmod 755 "$stage_dir/$PKG_NAME"
    [[ -f "$PROJECT_DIR/example.settings.toml" ]] && \
        cp "$PROJECT_DIR/example.settings.toml" "$stage_dir/settings.toml.example"

    local tgz_file="$MAC_DIR/${PKG_NAME}_${PKG_VERSION}_${arch_label}.tar.gz"
    tar -czf "$tgz_file" -C "$stage_dir" .
    rm -rf "$stage_dir"

    # Print SHA256 – needed for the Homebrew formula
    local sha256
    sha256="$(shasum -a 256 "$tgz_file" | awk '{print $1}')"
    log "macOS tar.gz created: $tgz_file"
    log "  SHA256 ($arch_label): $sha256"
    echo "$arch_label $sha256" >> "$MAC_DIR/sha256sums.txt"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
mkdir -p "$LINUX_DEB_DIR" "$LINUX_RPM_DIR" "$WIN_DIR" "$MAC_DIR"

# Check Docker once upfront if any Linux or Windows targets are requested.
if [[ ${#LINUX_TARGETS[@]} -gt 0 || ${#WIN_TARGETS[@]} -gt 0 ]]; then
    check_docker
fi

# ----- Linux -----
for target in "${LINUX_TARGETS[@]:-}"; do
    [[ -z "$target" ]] && continue
    log "=== Linux target: $target ==="
    case "$PKG_TYPE" in
        deb)   build_deb "$target" ;;
        rpm)   build_rpm "$target" ;;
        targz) warn "targz not applicable to Linux targets" ;;
        zip)   warn "zip not applicable to Linux targets" ;;
        all)
            build_deb "$target" || warn "DEB build failed for $target"
            build_rpm "$target" || warn "RPM build failed for $target"
            ;;
    esac
done

# ----- Windows -----
for target in "${WIN_TARGETS[@]:-}"; do
    [[ -z "$target" ]] && continue
    log "=== Windows target: $target ==="
    case "$PKG_TYPE" in
        zip|all) build_win_zip "$target" || warn "Windows ZIP failed for $target" ;;
        deb|rpm) warn "deb/rpm not applicable to Windows targets" ;;
    esac
done

# ----- macOS -----
for target in "${MAC_TARGETS[@]:-}"; do
    [[ -z "$target" ]] && continue
    log "=== macOS target: $target ==="
    case "$PKG_TYPE" in
        targz|all) build_mac_targz "$target" || warn "macOS tar.gz failed for $target" ;;
        deb|rpm)   warn "deb/rpm not applicable to macOS targets" ;;
    esac
done

# ----- Summary -----
log "=== Build complete. Packages in $RELEASE_DIR ==="
find "$RELEASE_DIR" \( -name "*.deb" -o -name "*.rpm" -o -name "*.zip" -o -name "*.tar.gz" \) \
    2>/dev/null | sort | while read -r f; do
    echo "  $f"
done

# ---------------------------------------------------------------------------
# Cleanup – remove Cargo build artifacts to reclaim disk space.
# ---------------------------------------------------------------------------
log "=== Cleaning up Cargo build artifacts …"
ALL_TRIPLES=()
for t in "${LINUX_TARGETS[@]:-}" "${WIN_TARGETS[@]:-}" "${MAC_TARGETS[@]:-}"; do
    [[ -z "$t" ]] && continue
    ALL_TRIPLES+=("$(get_triple "$t")")
done

for triple in "${ALL_TRIPLES[@]:-}"; do
    [[ -z "$triple" ]] && continue
    target_dir="$PROJECT_DIR/target/$triple/release"
    if [[ -d "$target_dir" ]]; then
        find "$target_dir" -maxdepth 1 -type f -delete
        rm -rf "$target_dir/incremental" "$target_dir/build" "$target_dir/deps"
        log "  Cleaned: target/$triple/release"
    fi
done

log "=== Cleanup done. Disk usage of release dir ==="
du -sh "$RELEASE_DIR" 2>/dev/null || true
