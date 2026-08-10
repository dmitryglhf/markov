#!/usr/bin/env bash
set -euo pipefail

##############################################################################
# Packages the built macOS artifacts and publishes them as a GitHub release.
#
#   scripts/publish.sh              package and publish
#   scripts/publish.sh --dry-run    package only, print what would be uploaded
#
# Expects a desktop bundle and a release CLI to exist already:
#   just make-ui
#
# Assets carry no version in their names: that is what lets GitHub serve the
# newest release under a fixed /releases/latest/download/ URL, which is the
# address scripts/install.sh reads.
#
# Authentication comes from `gh auth login` (or GH_TOKEN in the environment).
#
# Environment:
#   MARKOV_VERSION  version to publish (default: ui/desktop/package.json)
#   MARKOV_REPO     GitHub repository to publish to
##############################################################################

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"

REPO="${MARKOV_REPO:-dmitryglhf/markov}"
STAGE="ui/desktop/out/release"

DRY_RUN=false
[ "${1:-}" = "--dry-run" ] && DRY_RUN=true

die() {
  echo "error: $*" >&2
  exit 1
}

command -v gh >/dev/null 2>&1 || die "the GitHub CLI is required: brew install gh"

VERSION="${MARKOV_VERSION:-$(sed -n 's/^  "version": "\(.*\)",$/\1/p' ui/desktop/package.json)}"
[ -n "$VERSION" ] || die "could not read a version from ui/desktop/package.json"
TAG="v${VERSION#v}"

# An artifact nobody can rebuild is not worth publishing.
[ -z "$(git status --porcelain)" ] || die "working tree is dirty — commit or stash first"
COMMIT="$(git rev-parse HEAD)"

case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin"; FORGE_ARCH="arm64" ;;
  x86_64) TARGET="x86_64-apple-darwin"; FORGE_ARCH="x64" ;;
  *) die "no packaging for $(uname -m)" ;;
esac

DESKTOP_BUILT="ui/desktop/out/make/zip/darwin/$FORGE_ARCH/Markov-darwin-$FORGE_ARCH-$VERSION.zip"
CLI_BUILT="target/release/markov"
[ -f "$DESKTOP_BUILT" ] || die "no desktop bundle at $DESKTOP_BUILT — run: just make-ui"
[ -f "$CLI_BUILT" ] || die "no CLI at $CLI_BUILT — run: just release-binary"

echo "Packaging Markov $VERSION from $(git rev-parse --short HEAD) ($TARGET)"

rm -rf "$STAGE"
mkdir -p "$STAGE"
ln -f "$DESKTOP_BUILT" "$STAGE/markov-desktop-$TARGET.zip"
# gzip stamps the current time into its header unless told not to, which would
# give the same binary a different checksum on every packaging run.
tar -cf - -C "$(dirname "$CLI_BUILT")" markov | gzip -n > "$STAGE/markov-cli-$TARGET.tar.gz"

# The installer travels with what it installs, so the one-liner needs no
# checkout and no credentials.
cp scripts/install.sh "$STAGE/install.sh"
(cd "$STAGE" && shasum -a 256 -- *.zip *.tar.gz > SHA256SUMS)

if $DRY_RUN; then
  echo
  echo "Would publish $TAG to $REPO:"
  (cd "$STAGE" && ls -1 | sed 's/^/  /')
  exit 0
fi

# A tag can only point at a commit GitHub already has.
gh api "repos/$REPO/commits/$COMMIT" --jq .sha >/dev/null 2>&1 ||
  die "$REPO does not have $COMMIT — push it first"

# Assets first and sums last: a reader that already has SHA256SUMS must never
# find an archive that has not landed yet.
ASSETS=("$STAGE/markov-desktop-$TARGET.zip" "$STAGE/markov-cli-$TARGET.tar.gz" "$STAGE/install.sh")

echo
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "Updating existing release $TAG"
  gh release upload "$TAG" --repo "$REPO" --clobber "${ASSETS[@]}"
  gh release upload "$TAG" --repo "$REPO" --clobber "$STAGE/SHA256SUMS"
else
  echo "Creating release $TAG"
  # Drafted first so the release becomes visible only once every asset is up.
  gh release create "$TAG" --repo "$REPO" --target "$COMMIT" --draft \
    --title "Markov $VERSION" --notes "Markov $VERSION"
  gh release upload "$TAG" --repo "$REPO" "${ASSETS[@]}" "$STAGE/SHA256SUMS"
  gh release edit "$TAG" --repo "$REPO" --draft=false --latest
fi

echo
echo "Published $TAG"
echo "  https://github.com/$REPO/releases/tag/$TAG"
echo "  curl -fsSL https://github.com/$REPO/releases/latest/download/install.sh | bash"
