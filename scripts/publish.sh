#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Packages the built macOS artifacts and uploads them to the GitLab generic
# package registry, under both the version and the `latest` channel.
#
#   scripts/publish.sh              package and upload
#   scripts/publish.sh --dry-run    package only, print what would be uploaded
#
# Expects a desktop bundle and a release CLI to exist already:
#   just make-ui
#
# Token: GITLAB_TOKEN_WRITE, from the environment or from .env at the repo
# root. It is passed through a curl config file rather than a command line
# argument, which would show up in `ps` for every user on the machine.
#
# Environment:
#   GITLAB_TOKEN_WRITE  token with the `api` scope
#   MARKOV_VERSION      version to publish (default: ui/desktop/package.json)
#   MARKOV_REGISTRY     package registry base
##############################################################################

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"

REGISTRY="${MARKOV_REGISTRY:-https://git.postgrespro.ru/api/v4/projects/askpostgres%2Fmarkov/packages/generic/markov}"
TARGET="aarch64-apple-darwin"
STAGE="ui/desktop/out/release"

DRY_RUN=false
[ "${1:-}" = "--dry-run" ] && DRY_RUN=true

die() {
  echo "error: $*" >&2
  exit 1
}

VERSION="${MARKOV_VERSION:-$(sed -n 's/^  "version": "\(.*\)",$/\1/p' ui/desktop/package.json)}"
[ -n "$VERSION" ] || die "could not read a version from ui/desktop/package.json"

# An artifact nobody can rebuild is not worth publishing.
[ -z "$(git status --porcelain)" ] || die "working tree is dirty — commit or stash first"
COMMIT="$(git rev-parse --short HEAD)"

DESKTOP_BUILT="ui/desktop/out/make/zip/darwin/arm64/Markov-darwin-arm64-$VERSION.zip"
CLI_BUILT="target/release/markov"
[ -f "$DESKTOP_BUILT" ] || die "no desktop bundle at $DESKTOP_BUILT — run: just make-ui"
[ -f "$CLI_BUILT" ] || die "no CLI at $CLI_BUILT — run: just release-binary"

echo "Packaging Markov $VERSION from $COMMIT"

rm -rf "$STAGE"
mkdir -p "$STAGE/$VERSION" "$STAGE/latest"
ln -f "$DESKTOP_BUILT" "$STAGE/$VERSION/markov-desktop-$VERSION-$TARGET.zip"
tar -czf "$STAGE/$VERSION/markov-cli-$VERSION-$TARGET.tar.gz" -C "$(dirname "$CLI_BUILT")" markov

# `latest` is the same bytes under another name: hard links keep one copy on
# disk and save compressing the CLI a second time.
for pair in desktop:zip cli:tar.gz; do
  ln -f "$STAGE/$VERSION/markov-${pair%%:*}-$VERSION-$TARGET.${pair#*:}" \
        "$STAGE/latest/markov-${pair%%:*}-latest-$TARGET.${pair#*:}"
done

for channel in "$VERSION" latest; do
  (cd "$STAGE/$channel" && shasum -a 256 -- *.zip *.tar.gz > SHA256SUMS)
done

if $DRY_RUN; then
  echo
  echo "Would upload to $REGISTRY:"
  (cd "$STAGE" && find . -type f | sed 's|^\./|  |' | sort)
  exit 0
fi

TOKEN="${GITLAB_TOKEN_WRITE:-}"
if [ -z "$TOKEN" ] && [ -f .env ]; then
  TOKEN="$(sed -n 's/^GITLAB_TOKEN_WRITE=//p' .env | tail -1 | tr -d "\"'")"
fi
[ -n "$TOKEN" ] || die "GITLAB_TOKEN_WRITE is not set and .env does not carry it"

CURL_CONFIG="$(mktemp)"
chmod 600 "$CURL_CONFIG"
trap 'rm -f "$CURL_CONFIG"' EXIT
printf 'header = "PRIVATE-TOKEN: %s"\n' "$TOKEN" > "$CURL_CONFIG"

echo
for channel in "$VERSION" latest; do
  for file in "$STAGE/$channel"/*; do
    name="$(basename "$file")"
    printf '  %-52s ' "$channel/$name"
    curl --config "$CURL_CONFIG" --fail --silent --show-error \
      --upload-file "$file" "$REGISTRY/$channel/$name" >/dev/null
    echo "ok"
  done
done

echo
echo "Published $VERSION ($COMMIT)"
echo "  https://git.postgrespro.ru/askpostgres/markov/-/packages"
