---
name: release
description: Prepare a gps-to-mqtt release — version, changelog, dependency updates and audit, packages (deb/rpm/zip/tar.gz), Docker image, licence.
---

# Release

1. `cargo update`, then check direct dependencies for new majors
   (`cargo search <crate> --limit 1`). Raise the minimums in `Cargo.toml`.
   Keep `rust-version` honest (`cargo update` warns when it holds versions back).
2. `cargo audit`. Fix by updating. Add exceptions to `.cargo/audit.toml` only
   with a written reason, and remove them when upstream fixes land.
3. `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
4. Bump `version` in `Cargo.toml` and add a `CHANGELOG.md` entry (contract
   changes first, then features, fixes and security).
5. Hardware smoke test (hardware-validation skill), or state explicitly that it was skipped.
6. `docker build -t gps-to-mqtt:<v> .` and `docker run --rm gps-to-mqtt:<v> --check-config`.
7. `./scripts/build_packages.sh [--platform …]`, which writes to `release/<version>/`
   (gitignored). Packages ship `LICENSE` (Debian `copyright`, RPM `%license`).
   Signing material and repository credentials stay in their private locations.
8. The Homebrew formula lives in `../../homebrew/homebrew-g86racing`; update its
   URL, sha256 and licence there (separate repository).

Licence: GPL-3.0-only (`LICENSE`, `Cargo.toml`, `PKG_LICENSE` in the build script).
