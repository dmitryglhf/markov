#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Markov uninstaller.
#
#   curl -fsSL https://github.com/dmitryglhf/markov/releases/download/stable/deinstall.sh | bash
#   curl -fsSL <same url> | bash -s -- --keep-data
#   curl -fsSL <same url> | bash -s -- --yes
#
# Removes the desktop app, the CLI and, unlike `install.sh --uninstall`,
# the settings, credentials and sessions as well. Everything it touches lives
# in the user's own home, so no administrator rights are needed.
#
# Environment:
#   MARKOV_APP_DIR      where the app was put (default: ~/Applications, ~/.local/share on Linux)
#   MARKOV_INSTALL_DIR  where the CLI was put (default: ~/.local/bin)
#   MARKOV_OS           darwin|linux, overrides detection
##############################################################################

KEEP_DATA=false
ASSUME_YES=false
for arg in "$@"; do
  case "$arg" in
    --keep-data) KEEP_DATA=true ;;
    -y|--yes) ASSUME_YES=true ;;
    -h|--help)
      # $0 is not a readable path when the script arrives through a pipe.
      cat <<'USAGE'
Markov uninstaller.

  deinstall.sh              remove the app, the CLI, settings and sessions
  deinstall.sh --keep-data  remove the app and the CLI, keep settings
  deinstall.sh --yes        do not ask for confirmation

Environment: MARKOV_APP_DIR, MARKOV_INSTALL_DIR, MARKOV_OS
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
  OS="linux"
else
  case "$(uname -s)" in
    Darwin) OS="darwin" ;;
    Linux) OS="linux" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *) die "unsupported system '$(uname -s)'" ;;
  esac
fi

[ "$OS" = "windows" ] && die "on Windows use deinstall.ps1"

INSTALL_DIR="${MARKOV_INSTALL_DIR:-$HOME/.local/bin}"
LINK_PATH="$INSTALL_DIR/markov"

if [ "$OS" = "darwin" ]; then
  APP_DIR="${MARKOV_APP_DIR:-$HOME/Applications}"
  APP_PATH="$APP_DIR/Markov.app"
  BUNDLED_BINARY="$APP_PATH/Contents/Resources/bin/markov"
  APP_EXECUTABLE="$APP_PATH/Contents/MacOS/Markov"
  # Electron keeps its own store under the product name, next to nothing else
  # of ours.
  DESKTOP_DATA="$HOME/Library/Application Support/Markov"
else
  APP_DIR="${MARKOV_APP_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}}"
  APP_PATH="$APP_DIR/markov-desktop"
  BUNDLED_BINARY="$APP_PATH/resources/bin/markov"
  APP_EXECUTABLE="$APP_PATH/Markov"
  DESKTOP_DATA="${XDG_CONFIG_HOME:-$HOME/.config}/Markov"
fi

DESKTOP_ENTRY="${XDG_DATA_HOME:-$HOME/.local/share}/applications/markov.desktop"

# The CLI reads XDG paths on macOS too, so these are not the Library locations
# a Mac user would expect.
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/markov"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/markov"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/markov"

##############################################################################
# What is actually here
##############################################################################

PROGRAM_TARGETS=()
[ -d "$APP_PATH" ] && PROGRAM_TARGETS+=("$APP_PATH")
[ -f "$DESKTOP_ENTRY" ] && PROGRAM_TARGETS+=("$DESKTOP_ENTRY")

# A link pointing anywhere else is somebody else's markov, and a plain file is
# a CLI-only install of ours.
if [ -L "$LINK_PATH" ]; then
  [ "$(readlink "$LINK_PATH")" = "$BUNDLED_BINARY" ] && PROGRAM_TARGETS+=("$LINK_PATH")
elif [ -f "$LINK_PATH" ]; then
  PROGRAM_TARGETS+=("$LINK_PATH")
fi

DATA_TARGETS=()
if ! $KEEP_DATA; then
  for dir in "$CONFIG_DIR" "$DATA_DIR" "$STATE_DIR" "$DESKTOP_DATA"; do
    [ -d "$dir" ] && DATA_TARGETS+=("$dir")
  done
fi

if [ ${#PROGRAM_TARGETS[@]} -eq 0 ] && [ ${#DATA_TARGETS[@]} -eq 0 ]; then
  echo "Nothing to remove — markov is not installed here."
  exit 0
fi

if [ ${#PROGRAM_TARGETS[@]} -gt 0 ] && pgrep -f "$APP_EXECUTABLE" >/dev/null 2>&1; then
  die "Markov is running — quit it first"
fi

##############################################################################
# Confirm
##############################################################################

echo "About to remove:"
for target in ${PROGRAM_TARGETS[@]+"${PROGRAM_TARGETS[@]}"} ${DATA_TARGETS[@]+"${DATA_TARGETS[@]}"}; do
  echo "  $target"
done

if [ ${#DATA_TARGETS[@]} -gt 0 ]; then
  echo
  echo "This includes your settings, provider keys and every saved session."
  echo "Pass --keep-data to remove the program only."
fi

if ! $ASSUME_YES; then
  echo
  # Piped through curl, stdin is the script itself, so the answer has to come
  # from the terminal directly. It can be openable and still unreadable, with
  # no controlling terminal behind it.
  reply=""
  [ -r /dev/tty ] || die "no terminal to confirm on — rerun with --yes"
  printf "Continue? [y/N] "
  read -r reply </dev/tty 2>/dev/null || { echo; die "no terminal to confirm on — rerun with --yes"; }
  case "$reply" in
    y|Y|yes|YES) ;;
    *) echo "Cancelled."; exit 0 ;;
  esac
fi

##############################################################################
# Remove
##############################################################################

for target in ${PROGRAM_TARGETS[@]+"${PROGRAM_TARGETS[@]}"} ${DATA_TARGETS[@]+"${DATA_TARGETS[@]}"}; do
  rm -rf "$target"
  echo "removed $target"
done

# Ours to make, ours to take back, but only while nobody else lives there.
if [ -d "$INSTALL_DIR" ] && [ -z "$(ls -A "$INSTALL_DIR" 2>/dev/null)" ]; then
  rmdir "$INSTALL_DIR" 2>/dev/null || true
fi

echo
echo "Done."
if $KEEP_DATA; then
  echo "Settings and sessions were left alone:"
  echo "  $CONFIG_DIR"
  echo "  $DATA_DIR"
  echo "  $STATE_DIR"
fi
# Shared with other agents, so never ours alone to delete.
if [ -d "$HOME/.agents" ]; then
  echo "Left in place: ~/.agents (plugins and skills, shared with other agents)"
fi
