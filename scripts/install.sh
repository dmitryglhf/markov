#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Markov installer.
#
#   curl -fsSL https://github.com/dmitryglhf/markov/releases/download/stable/install.sh | bash
#   curl -fsSL <same url> | bash -s -- --cli-only
#   curl -fsSL <same url> | bash -s -- --uninstall
#
# macOS gets the desktop app in ~/Applications and Linux in ~/.local/share, both
# with a `markov` symlink into the bundle, which keeps the CLI and the app on one
# version. Windows gets the CLI on its own; there install.ps1 is the supported
# route and the one that installs the app. Everything is checked against
# SHA256SUMS before it is unpacked.
#
# Installs into the user's own home, so no administrator rights are needed.
#
# Environment:
#   MARKOV_VERSION          exact version to install, e.g. 1.45.0
#   MARKOV_CHANNEL          stable|canary (default: stable)
#   MARKOV_REPO             GitHub repository releases are read from
#   MARKOV_BASE_URL         full release asset base, overrides the two above
#   MARKOV_APP_DIR          where the app goes (default: ~/Applications, ~/.local/share on Linux)
#   MARKOV_INSTALL_DIR      where the CLI goes (default: ~/.local/bin)
#   MARKOV_OS               darwin|linux|windows, overrides detection
#   MARKOV_LINUX_VARIANT    standard|musl (musl is detected)
##############################################################################

VERSION="${MARKOV_VERSION:-}"
CHANNEL="${MARKOV_CHANNEL:-stable}"
REPO="${MARKOV_REPO:-dmitryglhf/markov}"

CLI_ONLY=false
UNINSTALL=false
for arg in "$@"; do
  case "$arg" in
    --cli-only) CLI_ONLY=true ;;
    --uninstall) UNINSTALL=true ;;
    -h|--help)
      # $0 is not a readable path when the script arrives through a pipe.
      cat <<'USAGE'
Markov installer.

  install.sh              macOS and Linux: desktop app + CLI; Windows: CLI
  install.sh --cli-only   CLI only, no desktop app
  install.sh --uninstall  remove the app and the CLI, keep settings

Environment: MARKOV_VERSION, MARKOV_CHANNEL, MARKOV_REPO, MARKOV_BASE_URL, MARKOV_APP_DIR,
MARKOV_INSTALL_DIR, MARKOV_OS, MARKOV_LINUX_VARIANT, MARKOV_WINDOWS_VARIANT
USAGE
      exit 0
      ;;
    *) echo "error: unknown option '$arg'" >&2; exit 1 ;;
  esac
done

die() {
  echo "error: $*" >&2
  exit 1
}

##############################################################################
# Platform
##############################################################################

if [ -n "${MARKOV_OS:-}" ]; then
  OS="$MARKOV_OS"
elif [ -n "${WINDIR:-}" ] || [ -n "${windir:-}" ] || [ "${OSTYPE:-}" = "msys" ] || [ "${OSTYPE:-}" = "cygwin" ]; then
  OS="windows"
elif [ -f /proc/version ] && grep -qi "microsoft\|wsl" /proc/version 2>/dev/null; then
  # WSL is Linux whatever the current directory looks like.
  OS="linux"
else
  case "$(uname -s)" in
    Darwin) OS="darwin" ;;
    Linux) OS="linux" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *) die "unsupported system '$(uname -s)'" ;;
  esac
fi

ARCH="$(uname -m)"
# Under Rosetta `uname -m` answers for the translated process, not the machine,
# and would install the slower build on an Apple Silicon Mac.
if [ "$OS" = "darwin" ] && [ "$ARCH" = "x86_64" ] &&
   [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
  ARCH="arm64"
fi

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *) die "no build for $ARCH" ;;
esac

if [ "$OS" = "windows" ]; then
  CLI_NAME="markov.exe"
  INSTALL_DIR="${MARKOV_INSTALL_DIR:-${USERPROFILE:-$HOME}/markov}"
else
  CLI_NAME="markov"
  INSTALL_DIR="${MARKOV_INSTALL_DIR:-$HOME/.local/bin}"
fi

if [ "$OS" = "darwin" ]; then
  APP_DIR="${MARKOV_APP_DIR:-$HOME/Applications}"
  APP_NAME="Markov.app"
  APP_PATH="$APP_DIR/$APP_NAME"
  BUNDLED_BINARY="$APP_PATH/Contents/Resources/bin/markov"
  APP_EXECUTABLE="$APP_PATH/Contents/MacOS/Markov"
else
  APP_DIR="${MARKOV_APP_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}}"
  APP_NAME="markov-desktop"
  APP_PATH="$APP_DIR/$APP_NAME"
  BUNDLED_BINARY="$APP_PATH/resources/bin/markov"
  APP_EXECUTABLE="$APP_PATH/Markov"
fi

DESKTOP_ENTRY="${XDG_DATA_HOME:-$HOME/.local/share}/applications/markov.desktop"
LINK_PATH="$INSTALL_DIR/$CLI_NAME"

##############################################################################
# Uninstall
##############################################################################

remove_cli() {
  if [ -L "$LINK_PATH" ]; then
    # Only a link we made: one pointing anywhere else is somebody else's markov.
    if [ "$(readlink "$LINK_PATH")" = "$BUNDLED_BINARY" ]; then
      rm -f "$LINK_PATH"
      echo "removed $LINK_PATH"
    fi
  elif [ -f "$LINK_PATH" ]; then
    rm -f "$LINK_PATH"
    echo "removed $LINK_PATH"
  fi
}

running_instance() {
  [ "$OS" != "windows" ] && pgrep -f "$APP_EXECUTABLE" >/dev/null 2>&1
}

if $UNINSTALL; then
  running_instance && die "Markov is running — quit it first"
  remove_cli
  # There is no app on Windows, where $APP_PATH is a path that was never
  # installed and may well belong to something else.
  if [ "$OS" != "windows" ] && [ -d "$APP_PATH" ]; then
    rm -rf "$APP_PATH"
    echo "removed $APP_PATH"
  fi
  if [ "$OS" = "linux" ] && [ -f "$DESKTOP_ENTRY" ]; then
    rm -f "$DESKTOP_ENTRY"
    echo "removed $DESKTOP_ENTRY"
  fi
  echo
  echo "Settings and sessions were left alone:"
  echo "  ~/.config/markov"
  echo "  ~/.local/share/markov"
  [ "$OS" = "darwin" ] && echo "  ~/Library/Application Support/Markov"
  exit 0
fi

##############################################################################
# What to download
##############################################################################

case "$OS" in
  darwin)
    TARGET="$ARCH-apple-darwin"
    CLI_ARCHIVE="markov-cli-$TARGET.tar.gz"
    APP_ARCHIVE="markov-desktop-$TARGET.zip"
    ;;
  linux)
    VARIANT="${MARKOV_LINUX_VARIANT:-}"
    if [ -z "$VARIANT" ]; then
      if [ "${OSTYPE:-}" = "linux-musl" ] ||
         { command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; }; then
        VARIANT="musl"
      else
        VARIANT="standard"
      fi
    fi
    case "$VARIANT" in
      standard) TARGET="$ARCH-unknown-linux-gnu" ;;
      musl) TARGET="$ARCH-unknown-linux-musl" ;;
      *) die "unsupported MARKOV_LINUX_VARIANT '$VARIANT' (standard|musl)" ;;
    esac
    CLI_ARCHIVE="markov-cli-$TARGET.tar.gz"
    APP_ARCHIVE="markov-desktop-$TARGET.zip"
    # The desktop app is built for glibc on x86_64 only.
    if [ "$ARCH" != "x86_64" ] || [ "$VARIANT" = "musl" ]; then
      if ! $CLI_ONLY; then
        echo "no desktop build for $TARGET — installing the CLI only" >&2
      fi
      CLI_ONLY=true
    fi
    ;;
  windows)
    [ "$ARCH" = "x86_64" ] || die "Windows builds are x86_64 only"
    TARGET="x86_64-pc-windows-msvc"
    CLI_ARCHIVE="markov-cli-$TARGET.zip"
    CLI_ONLY=true
    ;;
esac

case "$CHANNEL" in
  stable|canary) ;;
  *) die "unsupported MARKOV_CHANNEL '$CHANNEL' (stable|canary)" ;;
esac

# Assets carry no version in their names, so a moving tag serves the current
# build. The channel is named outright rather than resolved through GitHub's
# "latest", which the canary release would otherwise claim on every push.
if [ -n "${MARKOV_BASE_URL:-}" ]; then
  BASE_URL="$MARKOV_BASE_URL"
  LABEL="$CHANNEL"
elif [ -n "$VERSION" ]; then
  BASE_URL="https://github.com/$REPO/releases/download/v${VERSION#v}"
  LABEL="${VERSION#v}"
else
  BASE_URL="https://github.com/$REPO/releases/download/$CHANNEL"
  LABEL="$CHANNEL"
fi

##############################################################################
# Download
##############################################################################

command -v curl >/dev/null 2>&1 || die "'curl' is required"

# Plain --retry covers timeouts and 5xx, but not a stream that breaks mid-transfer,
# which is how a two-hundred-megabyte download usually dies. --retry-all-errors does
# cover it, and exists since curl 7.71, so it is added only where it is understood —
# and only for the archives, because it would also sit retrying the 404 that tells
# us a release is simply not there.
RETRY=(--retry 3 --retry-delay 2)
RETRY_ARCHIVE=("${RETRY[@]}")
curl --retry-all-errors --version >/dev/null 2>&1 && RETRY_ARCHIVE+=(--retry-all-errors)

if command -v shasum >/dev/null 2>&1; then
  SUM_CMD="shasum -a 256"
elif command -v sha256sum >/dev/null 2>&1; then
  SUM_CMD="sha256sum"
else
  die "'shasum' or 'sha256sum' is required"
fi

running_instance && die "Markov is running — quit it first, then run this again"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

fetch() {
  curl -fSL "${RETRY_ARCHIVE[@]}" --progress-bar "$BASE_URL/$1" -o "$WORK_DIR/$1" ||
    die "could not download $1"
}

verify() {
  local listed
  listed="$(grep " $1\$" "$WORK_DIR/SHA256SUMS" || true)"
  # An archive missing from the list is never silently accepted.
  [ -n "$listed" ] || die "$1 is not listed in SHA256SUMS"
  (cd "$WORK_DIR" && printf '%s\n' "$listed" | $SUM_CMD -c -) ||
    die "checksum mismatch — the download is not what the release published"
}

echo "Installing Markov $LABEL ($TARGET)"
curl -fsSL "${RETRY[@]}" "$BASE_URL/SHA256SUMS" -o "$WORK_DIR/SHA256SUMS" ||
  die "no release '$LABEL' at $BASE_URL"

if $CLI_ONLY; then
  fetch "$CLI_ARCHIVE"
  verify "$CLI_ARCHIVE"
else
  fetch "$APP_ARCHIVE"
  verify "$APP_ARCHIVE"
fi

##############################################################################
# Install
##############################################################################

mkdir -p "$INSTALL_DIR"

if $CLI_ONLY; then
  case "$CLI_ARCHIVE" in
    *.zip)
      command -v unzip >/dev/null 2>&1 || die "'unzip' is required"
      unzip -q -o "$WORK_DIR/$CLI_ARCHIVE" -d "$WORK_DIR/unpacked"
      ;;
    *)
      mkdir -p "$WORK_DIR/unpacked"
      tar -xzf "$WORK_DIR/$CLI_ARCHIVE" -C "$WORK_DIR/unpacked"
      ;;
  esac

  BINARY="$(find "$WORK_DIR/unpacked" -type f -name "$CLI_NAME" | head -1)"
  [ -n "$BINARY" ] || die "$CLI_ARCHIVE does not contain $CLI_NAME"

  # A stale symlink into a removed app would otherwise swallow the new binary.
  rm -f "$LINK_PATH"
  mv "$BINARY" "$LINK_PATH"
  chmod +x "$LINK_PATH"
  [ "$OS" = "darwin" ] && xattr -dr com.apple.quarantine "$LINK_PATH" 2>/dev/null || true

  echo
  echo "Markov installed."
  echo "  cli: $LINK_PATH"
elif [ "$OS" = "linux" ]; then
  command -v unzip >/dev/null 2>&1 || die "'unzip' is required"
  unzip -q -o "$WORK_DIR/$APP_ARCHIVE" -d "$WORK_DIR/unpacked"

  BUILT="$(find "$WORK_DIR/unpacked" -maxdepth 2 -type f -name Markov | head -1)"
  [ -n "$BUILT" ] || die "$APP_ARCHIVE does not contain the Markov executable"

  mkdir -p "$APP_DIR"
  rm -rf "$APP_PATH"
  mv "$(dirname "$BUILT")" "$APP_PATH"
  chmod +x "$APP_EXECUTABLE" "$BUNDLED_BINARY"
  ln -sfn "$BUNDLED_BINARY" "$LINK_PATH"

  # Chromium sandboxes renderers with user namespaces and only falls back to the
  # setuid helper, which we cannot install without root. Where namespaces are
  # withheld from unconfined binaries the app would abort on start instead.
  SANDBOX_FLAG=""
  if { [ -r /proc/sys/user/max_user_namespaces ] &&
       [ "$(cat /proc/sys/user/max_user_namespaces)" = "0" ]; } ||
     { [ -r /proc/sys/kernel/unprivileged_userns_clone ] &&
       [ "$(cat /proc/sys/kernel/unprivileged_userns_clone)" = "0" ]; } ||
     { [ -r /proc/sys/kernel/apparmor_restrict_unprivileged_userns ] &&
       [ "$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns)" = "1" ]; }; then
    SANDBOX_FLAG=" --no-sandbox"
  fi

  mkdir -p "$(dirname "$DESKTOP_ENTRY")"
  cat >"$DESKTOP_ENTRY" <<ENTRY
[Desktop Entry]
Type=Application
Name=Markov
Comment=AI agent
Exec=$APP_EXECUTABLE$SANDBOX_FLAG %U
Icon=$APP_PATH/resources/images/icon.png
Categories=Development;
Terminal=false
StartupWMClass=Markov
ENTRY

  echo
  echo "Markov $LABEL installed."
  echo "  app: $APP_PATH"
  echo "  cli: $LINK_PATH -> $BUNDLED_BINARY"
  echo "  menu entry: $DESKTOP_ENTRY"
  if [ -n "$SANDBOX_FLAG" ]; then
    echo
    echo "This kernel withholds user namespaces from unconfined programs, so the"
    echo "menu entry starts the app with --no-sandbox. Installing the .deb or .rpm"
    echo "instead keeps the Chromium sandbox on."
  fi
else
  command -v ditto >/dev/null 2>&1 || die "'ditto' is required"
  ditto -x -k "$WORK_DIR/$APP_ARCHIVE" "$WORK_DIR/unpacked"
  [ -d "$WORK_DIR/unpacked/$APP_NAME" ] || die "$APP_ARCHIVE does not contain $APP_NAME"

  # Harmless after a curl download, which sets no quarantine; it matters when
  # the archive reached this machine some other way, because an unnotarized app
  # is something Gatekeeper refuses.
  xattr -dr com.apple.quarantine "$WORK_DIR/unpacked/$APP_NAME" 2>/dev/null || true

  mkdir -p "$APP_DIR"
  rm -rf "$APP_PATH"
  mv "$WORK_DIR/unpacked/$APP_NAME" "$APP_PATH"
  ln -sfn "$BUNDLED_BINARY" "$LINK_PATH"

  installed="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PATH/Contents/Info.plist" 2>/dev/null || echo "$LABEL")"
  echo
  echo "Markov $installed installed."
  echo "  app: $APP_PATH"
  echo "  cli: $LINK_PATH -> $BUNDLED_BINARY"
fi

if [ "$OS" = "windows" ]; then
  echo
  echo "For the desktop app, use install.ps1 from PowerShell:"
  echo "  irm $BASE_URL/install.ps1 | iex"
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo
    echo "$INSTALL_DIR is not on your PATH. Add it to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

echo
echo "Updates are not automatic — run this installer again for a new version."
