# Commands that exist only in this fork. Kept out of Justfile so upstream's
# recipes and ours never share a line, and a merge has nothing to arbitrate.
# Imported from Justfile; run them the usual way, e.g. `just markov-install`.

# where `just markov-install` puts the binary
INSTALL_DIR := env("MARKOV_INSTALL_DIR", home_directory() / ".local/bin")

# ----------------------------------------------------------------------
# Build the CLI
# ----------------------------------------------------------------------

# Upstream's release-binary builds their crate and their binary; this one is
# the shipped product.
# Release build of the CLI
markov-release:
    @echo "Building release version..."
    cargo build --release -p markov-cli --bin markov
    @just markov-copy

# Debug build of the CLI, same features as the release
markov-debug:
    @echo "Building debug version..."
    cargo build -p markov-cli --bin markov
    @echo "Built ./target/debug/markov"

# Build for Intel Mac
markov-release-intel:
    @echo "Building release version for Intel Mac..."
    cargo build --release --target x86_64-apple-darwin -p markov-cli --bin markov
    @just markov-copy-intel

# Build Windows executable on a Windows host
[unix]
markov-release-windows:
    @echo "just markov-release-windows requires a Windows host because Goose Windows releases build the MSVC target. Use .github/workflows/bundle-windows.yml for CI builds."
    @exit 1

[windows]
markov-release-windows:
    @powershell.exe -NoProfile -ExecutionPolicy Bypass -Command 'rustup target add x86_64-pc-windows-msvc; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; cargo build --release --target x86_64-pc-windows-msvc -p markov-cli --bin markov; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Write-Host "Windows executable created at ./target/x86_64-pc-windows-msvc/release/markov.exe"'

# ----------------------------------------------------------------------
# Stage the CLI inside the desktop bundle (forge picks up src/bin as extraResource)
# ----------------------------------------------------------------------

markov-copy BUILD_MODE="release":
    @rm -f ./ui/desktop/src/bin/goosed
    @if [ -f ./target/{{BUILD_MODE}}/markov ]; then \
        echo "Copying markov CLI binary from target/{{BUILD_MODE}}..."; \
        rm -f ./ui/desktop/src/bin/markov; \
        cp -p ./target/{{BUILD_MODE}}/markov ./ui/desktop/src/bin/; \
    else \
        echo "markov CLI binary not found in target/{{BUILD_MODE}}"; \
        exit 1; \
    fi

# Copy binary command for Intel build
markov-copy-intel:
    @rm -f ./ui/desktop/src/bin/goosed
    @if [ -f ./target/x86_64-apple-darwin/release/markov ]; then \
        echo "Copying Intel markov CLI binary to ui/desktop/src/bin..."; \
        rm -f ./ui/desktop/src/bin/markov; \
        cp -p ./target/x86_64-apple-darwin/release/markov ./ui/desktop/src/bin/; \
    else \
        echo "Intel markov CLI binary not found."; \
        exit 1; \
    fi

# Copy Windows binary command on a Windows host
[unix]
markov-copy-windows:
    @echo "just markov-copy-windows requires a Windows host because it copies the MSVC build output."
    @exit 1

[windows]
markov-copy-windows:
    @powershell.exe -NoProfile -ExecutionPolicy Bypass -Command 'if (Test-Path ./target/x86_64-pc-windows-msvc/release/markov.exe) { \
        Write-Host "Copying Windows binary to ui/desktop/src/bin..."; \
        New-Item -ItemType Directory -Force "./ui/desktop/src/bin" | Out-Null; \
        Remove-Item -Path "./ui/desktop/src/bin/markov.exe" -Force -ErrorAction SilentlyContinue; \
        Copy-Item -Path "./target/x86_64-pc-windows-msvc/release/markov.exe" -Destination "./ui/desktop/src/bin/" -Force; \
    } else { \
        Write-Host "Windows binary not found." -ForegroundColor Red; \
        exit 1; \
    }'

# ----------------------------------------------------------------------
# Put it on PATH
# ----------------------------------------------------------------------

# Build the release binary and put it on PATH as `markov`
[unix]
markov-install: markov-release
    #!/usr/bin/env bash
    set -euo pipefail

# Remove the binary `just markov-install` put on PATH
[unix]
markov-uninstall:
    #!/usr/bin/env bash
    set -euo pipefail

# ----------------------------------------------------------------------
# Desktop
#
# Upstream's run-ui/make-ui/package-ui chain builds `goose` and stages it as
# ui/desktop/src/bin/goose, but the app looks for `markov` (gooseServe.ts).
# These twins run the same steps against our binary.
# ----------------------------------------------------------------------

# Run the desktop app against a fresh release build
markov-run-ui: markov-release
    @echo "Running UI..."
    cd ui/desktop && pnpm install && pnpm run start-gui

# Same, with Playwright debugging in a throwaway GOOSE_PATH_ROOT
markov-run-ui-playwright:
    #!/usr/bin/env sh
    just markov-release
    echo "Running UI with Playwright debugging..."
    RUN_DIR="$HOME/markov-runs/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$RUN_DIR"
    echo "Using isolated directory: $RUN_DIR"
    cd ui/desktop && ENABLE_PLAYWRIGHT=true GOOSE_PATH_ROOT="$RUN_DIR" pnpm run start-gui

# Run the desktop app on a Windows host
[windows]
markov-run-ui-windows: markov-release-windows markov-copy-windows
    @powershell.exe -Command "Write-Host 'Running UI...'; Set-Location ui/desktop; pnpm install; pnpm run start-gui"

# Run the desktop app on a Windows host
[unix]
markov-run-ui-windows:
    @echo "just markov-run-ui-windows requires a Windows host."
    @exit 1

# Run the desktop app against a debug build
markov-run-dev:
    @echo "Building development version..."
    cargo build -p markov-cli --bin markov
    @just markov-copy debug
    @echo "Running UI..."
    cd ui/desktop && pnpm run start-gui

# Run the desktop app with main-process debugging
markov-debug-ui-main-process: markov-release
    @echo "Starting UI with main process debugging enabled"
    cd ui/desktop && pnpm install && pnpm run start-gui-debug

# Bundle the desktop app
markov-make-ui: markov-release
    cd ui/desktop && pnpm run bundle:default

# Bundle for Intel Mac
markov-make-ui-intel: markov-release-intel
    cd ui/desktop && pnpm run bundle:intel

# Bundle on a Windows host
[windows]
markov-make-ui-windows: markov-release-windows markov-copy-windows
    @powershell.exe -NoProfile -ExecutionPolicy Bypass -Command 'Set-Location ui/desktop; $env:ELECTRON_PLATFORM="win32"; node scripts/prepare-platform-binaries.js; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; pnpm run make --platform=win32 --arch=x64; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Write-Host "Windows package build complete!"'

# Bundle on a Windows host
[unix]
markov-make-ui-windows:
    @echo "just markov-make-ui-windows requires a Windows host because the MSVC target is built there. Use .github/workflows/bundle-windows.yml for CI builds."
    @exit 1

# Package the macOS app with ad-hoc signing (entitlements are needed for mic access)
markov-package-ui: markov-release
    @echo "Packaging desktop app..."
    cd ui/desktop && pnpm install && pnpm run package
    @echo "Signing with entitlements..."
    codesign --force --deep --sign - --entitlements ui/desktop/entitlements.plist ui/desktop/out/Markov-darwin-arm64/Markov.app
    @echo "Done! Launch with: open ui/desktop/out/Markov-darwin-arm64/Markov.app"

# ----------------------------------------------------------------------
# Local development
# ----------------------------------------------------------------------

# Run server
markov-serve:
    @echo "Running external ACP backend..."
    GOOSE_SERVER__SECRET_KEY="${GOOSE_SERVER__SECRET_KEY:-test}" cargo run -p markov-cli --bin markov -- serve --platform desktop --enable-scheduler --host 127.0.0.1 --port 3000

# Wipe local markov state (config, secrets, sessions, logs) to test setup from scratch. Pass "force" to skip the prompt
[unix]
markov-reset CONFIRM="":
    #!/usr/bin/env bash
    set -euo pipefail

# ----------------------------------------------------------------------
# Repository setup
# ----------------------------------------------------------------------

# Git ships no per-file `ours`/`theirs`, so without this the attributes in
# .gitattributes quietly decay into an ordinary three-way merge.
# Register the merge drivers .gitattributes names; run once per clone
markov-setup-merge:
    git config merge.ours.name "keep our version of the file"
    git config merge.ours.driver true
    git config merge.theirs.name "take the upstream version of the file"
    git config merge.theirs.driver "cp -f %B %A"
    @echo "registered:"
    @git config --get-regexp '^merge\.'
