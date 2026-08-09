#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Markov installer for macOS (Apple Silicon).
#
#   curl -fsSL <registry>/install.sh | bash
#   curl -fsSL <registry>/install.sh | bash -s -- --uninstall
#
# Installs into the user's own home, so no administrator rights are needed:
# the app lands in ~/Applications and `markov` becomes a symlink into the
# app's bundled binary, which keeps the CLI and the app on one version.
#
# Environment:
#   MARKOV_VERSION      version to install, e.g. 1.45.0 (default: latest)
#   MARKOV_BASE_URL     package registry base
#   MARKOV_APP_DIR      where Markov.app goes (default: ~/Applications)
#   MARKOV_INSTALL_DIR  where the `markov` symlink goes (default: ~/.local/bin)
##############################################################################

VERSION="${MARKOV_VERSION:-latest}"
BASE_URL="${MARKOV_BASE_URL:-https://git.postgrespro.ru/api/v4/projects/askpostgres%2Fmarkov/packages/generic/markov}"
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
[ "$(uname -m)" = "arm64" ] || die "no build for $(uname -m) yet — Apple Silicon only"

for tool in curl shasum ditto; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' is required"
done

running_instance && die "Markov is running — quit it first, then run this again"

TARGET="aarch64-apple-darwin"
ARCHIVE="markov-desktop-$VERSION-$TARGET.zip"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "Downloading Markov $VERSION..."
curl -fsSL "$BASE_URL/$VERSION/SHA256SUMS" -o "$WORK_DIR/SHA256SUMS" ||
  die "no SHA256SUMS for version '$VERSION' at $BASE_URL"
curl -fSL --progress-bar "$BASE_URL/$VERSION/$ARCHIVE" -o "$WORK_DIR/$ARCHIVE" ||
  die "could not download $ARCHIVE"

echo "Verifying..."
(cd "$WORK_DIR" && grep " $ARCHIVE\$" SHA256SUMS | shasum -a 256 -c -) ||
  die "checksum mismatch — the download is not what the registry published"

ditto -x -k "$WORK_DIR/$ARCHIVE" "$WORK_DIR/unpacked"
[ -d "$WORK_DIR/unpacked/$APP_NAME" ] || die "$ARCHIVE does not contain $APP_NAME"

# Harmless after a curl download, which sets no quarantine; it matters when the
# archive reached this machine some other way, because the app is signed ad-hoc
# and Gatekeeper refuses those.
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
