# cosmic-disks justfile

# Default recipe
default:
    @just --list

# Build the project
build:
    cargo build

# Build in release mode
release:
    cargo build --release

# Run the application
run:
    cargo run

# Run with RUST_LOG output
debug:
    RUST_LOG=cosmic_disks=debug cargo run

# Watch for changes and hot-reload
watch:
    cargo watch -x run

# Check the project for errors
check:
    cargo check

# Run clippy lints
lint:
    cargo clippy -- -W clippy::all

# Format code
fmt:
    cargo fmt

# Build release + install everything system-wide.
# Must be run from inside `nix develop` with sudo -E to preserve LD_LIBRARY_PATH:
#   sudo -E just install
#   sudo -E just install prefix=/usr
install prefix="/usr/local": release
    #!/usr/bin/env bash
    set -euo pipefail
    # Binary (actual ELF, invoked by the launcher wrapper below)
    install -Dm755 target/release/cosmic-disks "{{prefix}}/bin/cosmic-disks-bin"
    # Launcher wrapper — sets nix store LD_LIBRARY_PATH so Wayland/Vulkan libs are found
    # when the app is launched from the desktop rather than the nix dev shell.
    printf '#!/bin/sh\nexport LD_LIBRARY_PATH="%s:$LD_LIBRARY_PATH"\nexec {{prefix}}/bin/cosmic-disks-bin "$@"\n' \
        "${LD_LIBRARY_PATH:-}" | install -m755 /dev/stdin "{{prefix}}/bin/cosmic-disks"
    # Polkit rules — always goes to the system-wide rules dir regardless of prefix
    install -Dm644 data/polkit/dev.krishnqs.CosmicDisks.rules \
        /usr/share/polkit-1/rules.d/dev.krishnqs.CosmicDisks.rules
    # Desktop entry
    install -Dm644 data/dev.krishnqs.CosmicDisks.desktop \
        "{{prefix}}/share/applications/dev.krishnqs.CosmicDisks.desktop"
    # App icon
    install -Dm644 resources/icons/hicolor/scalable/apps/icon.svg \
        "{{prefix}}/share/icons/hicolor/scalable/apps/dev.krishnqs.CosmicDisks.svg"
    echo "Installed cosmic-disks to {{prefix}}"

# Build a .deb package for Debian/Ubuntu installation.
# Must run inside `nix develop` (cargo-deb and build deps live there).
# The resulting .deb links against Nix store libs, so it only works on
# machines that have Nix installed at the same store paths.
#
# Usage:
#   just deb                      # build + package
#   sudo dpkg -i target/debian/cosmic-disks_*.deb
#   sudo dpkg -r cosmic-disks     # uninstall
deb: release
    cargo deb --no-build
    @echo ""
    @ls -lh target/debian/cosmic-disks_*.deb | tail -1

# Clean build artifacts
clean:
    cargo clean
