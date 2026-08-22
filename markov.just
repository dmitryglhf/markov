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

    dest="{{INSTALL_DIR}}"
    mkdir -p "$dest"

    # Copied beside the target and renamed into place: rename swaps the directory
    # entry, so a markov session running right now keeps the file it started with
    # instead of having it rewritten underneath.
    cp ./target/release/markov "$dest/.markov.new"
    chmod 755 "$dest/.markov.new"
    mv -f "$dest/.markov.new" "$dest/markov"

    # awk, because `markov --version` answers with a leading space
    version=$("$dest/markov" --version | awk '{print $1}')
    echo "Installed markov $version to $dest/markov"

    case ":$PATH:" in
        *":$dest:"*) ;;
        *) echo "Note: $dest is not on your PATH, so plain \`markov\` will not find it" ;;
    esac

    on_path=$(command -v markov || true)
    if [ -n "$on_path" ] && [ "$on_path" != "$dest/markov" ]; then
        echo "Note: \`markov\` on your PATH is $on_path, which wins over the copy just installed"
    fi

# Remove the binary `just markov-install` put on PATH
[unix]
markov-uninstall:
    #!/usr/bin/env bash
    set -euo pipefail

    dest="{{INSTALL_DIR}}"
    if [ -e "$dest/markov" ]; then
        rm -f "$dest/markov"
        echo "Removed $dest/markov"
    else
        echo "Nothing to remove at $dest/markov"
    fi


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

    config="${XDG_CONFIG_HOME:-$HOME/.config}/markov"
    data="${XDG_DATA_HOME:-$HOME/.local/share}/markov"
    state="${XDG_STATE_HOME:-$HOME/.local/state}/markov"
    desktop="$HOME/Library/Application Support/Markov"

    targets=()
    for path in "$config" "$data" "$state" "$desktop"; do
        [ -e "$path" ] && targets+=("$path")
    done

    if [ ${#targets[@]} -eq 0 ]; then
        echo "No markov directories on disk"
    else
        echo "About to remove:"
        printf '  %s\n' "${targets[@]}"
    fi
    # the fork kept the upstream keyring service name, secrets live under "goose"
    echo "Also clearing keyring secrets (service goose, account secrets)"

    if pgrep -x markov >/dev/null 2>&1; then
        echo "Warning: markov is running, quit it first or it will rewrite the state"
    fi

    if [ "{{CONFIRM}}" != "force" ]; then
        read -r -p "Proceed? [y/N] " reply
        case "$reply" in
            y|Y) ;;
            *) echo "Cancelled"; exit 0 ;;
        esac
    fi

    for path in "${targets[@]:-}"; do
        [ -n "$path" ] && rm -rf "$path"
    done

    if command -v security >/dev/null 2>&1; then
        security delete-generic-password -s goose -a secrets >/dev/null 2>&1 || true
    elif command -v secret-tool >/dev/null 2>&1; then
        secret-tool clear service goose username secrets >/dev/null 2>&1 || true
    fi

    echo "Done, next markov run starts the first-time setup"
    echo "Tip: GOOSE_PATH_ROOT=<dir> markov keeps a throwaway setup out of your real one"
