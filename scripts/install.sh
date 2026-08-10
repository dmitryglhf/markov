#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Markov installer for macOS.
#
#   curl -fsSL https://github.com/dmitryglhf/markov/releases/latest/download/install.sh | bash
#   curl -fsSL https://github.com/dmitryglhf/markov/releases/latest/download/install.sh | bash -s -- --uninstall
#
# Installs into the user's own home, so no administrator rights are needed:
# the app lands in ~/Applications and `markov` becomes a symlink into the
# app's bundled binary, which keeps the CLI and the app on one version.
#
# Environment:
#   MARKOV_VERSION      version to install, e.g. 1.45.0 (default: latest)
#   MARKOV_REPO         GitHub repository releases are read from
#   MARKOV_BASE_URL     full release asset base, overrides the two above
#   MARKOV_APP_DIR      where Markov.app goes (default: ~/Applications)
#   MARKOV_INSTALL_DIR  where the `markov` symlink goes (default: ~/.local/bin)
##############################################################################

VERSION="${MARKOV_VERSION:-latest}"
REPO="${MARKOV_REPO:-dmitryglhf/markov}"
APP_DIR="${MARKOV_APP_DIR:-$HOME/Applications}"
INSTALL_DIR="${MARKOV_INSTALL_DIR:-$HOME/.local/bin}"

APP_NAME="Markov.app"
APP_PATH="$APP_DIR/$APP_NAME"
LINK_PATH="$INSTALL_DIR/markov"
BUNDLED_BINARY="$APP_PATH/Contents/Resources/bin/markov"

die() {
  echo "error: $*" >&2
  exit 1
}

running_instance() {
  pgrep -f "$APP_PATH/Contents/MacOS/Markov" >/dev/null 2>&1
}

uninstall() {
  # Only a link we own: a symlink elsewhere in PATH is somebody else's markov.
  if [ -L "$LINK_PATH" ] && [ "$(readlink "$LINK_PATH")" = "$BUNDLED_BINARY" ]; then
    rm -f "$LINK_PATH"
    echo "removed $LINK_PATH"
  fi
  if [ -d "$APP_PATH" ]; then
    rm -rf "$APP_PATH"
    echo "removed $APP_PATH"
  fi
  echo
  echo "Settings and sessions were left alone:"
  echo "  ~/.config/markov"
  echo "  ~/.local/share/markov"
  echo "  ~/Library/Application Support/Markov"
}

if [ "${1:-}" = "--uninstall" ]; then
  running_instance && die "Markov is running — quit it first"
  uninstall
  exit 0
fi

[ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS"

for tool in curl shasum ditto; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is required"
done

running_instance && die "Markov is running — quit it first, then run this again"

ARCH="$(uname -m)"
# Under Rosetta `uname -m` answers for the translated process, not the machine,
# and would install the slower build on an Apple Silicon Mac.
if [ "$ARCH" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
  ARCH="arm64"
fi

case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) die "no build for $ARCH" ;;
esac

# Release assets carry no version in their names, which is what lets GitHub
# serve the newest release under a fixed `latest` URL.
if [ -n "${MARKOV_BASE_URL:-}" ]; then
  BASE_URL="$MARKOV_BASE_URL"
elif [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/$REPO/releases/latest/download"
else
  BASE_URL="https://github.com/$REPO/releases/download/v${VERSION#v}"
fi

ARCHIVE="markov-desktop-$TARGET.zip"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "Downloading Markov $VERSION ($TARGET)..."
curl -fsSL "$BASE_URL/SHA256SUMS" -o "$WORK_DIR/SHA256SUMS" ||
  die "no release '$VERSION' at $BASE_URL"
curl -fSL --progress-bar "$BASE_URL/$ARCHIVE" -o "$WORK_DIR/$ARCHIVE" ||
  die "could not download $ARCHIVE"

echo "Verifying..."
(cd "$WORK_DIR" && grep " $ARCHIVE\$" SHA256SUMS | shasum -a 256 -c -) ||
  die "checksum mismatch — the download is not what the release published"

ditto -x -k "$WORK_DIR/$ARCHIVE" "$WORK_DIR/unpacked"
[ -d "$WORK_DIR/unpacked/$APP_NAME" ] || die "$ARCHIVE does not contain $APP_NAME"

# Harmless after a curl download, which sets no quarantine; it matters when the
# archive reached this machine some other way, because an unnotarized app is
# something Gatekeeper refuses.
xattr -dr com.apple.quarantine "$WORK_DIR/unpacked/$APP_NAME" 2>/dev/null || true

mkdir -p "$APP_DIR" "$INSTALL_DIR"
rm -rf "$APP_PATH"
mv "$WORK_DIR/unpacked/$APP_NAME" "$APP_PATH"

ln -sfn "$BUNDLED_BINARY" "$LINK_PATH"

installed="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PATH/Contents/Info.plist" 2>/dev/null || echo "$VERSION")"
echo
echo "Markov $installed installed."
echo "  app: $APP_PATH"
echo "  cli: $LINK_PATH -> $BUNDLED_BINARY"

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
