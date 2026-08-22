#!/usr/bin/env bash
# Renders the body of the upstream-merge pull request.
#
# Run it while `git merge <tag>` is still unresolved: the conflict classes come
# from the index, which is wiped the moment the merge is committed. Prints
# markdown to stdout and nothing else, so the caller can redirect it into a file.
#
#     git merge --no-ff v1.47.0 || scripts/upstream-inventory.sh v1.47.0 > body.md

set -uo pipefail

# Every list here is sorted, and half of them are paired with comm.
# Collation must not depend on whose machine this runs on.
export LC_ALL=C

TAG="${1:?usage: upstream-inventory.sh <upstream-tag>}"
UPSTREAM="${UPSTREAM:-aaif-goose/goose}"

BASE="$(git merge-base HEAD "$TAG")"

# Files whose conflicts nobody resolves by hand — they get regenerated.
is_lockfile() {
    case "$(basename -- "$1")" in
        Cargo.lock|pnpm-lock.yaml|package-lock.json|yarn.lock|flake.lock) return 0 ;;
        *) return 1 ;;
    esac
}

conflict_blocks() {
    grep -c '^<<<<<<< ' -- "$1" 2>/dev/null || echo 0
}

# Unmerged paths by class. DU is "we deleted, upstream modified" — the class that
# silently comes back if you reach for `git add -A` without thinking.
unmerged() { git status --porcelain | awk -v c="$1" '$1 == c { print $2 }'; }

# Newline-delimited rather than arrays: this has to stay runnable under the
# bash 3.2 that ships with macOS, where `mapfile` does not exist.
uu="$(unmerged UU)"
du="$(unmerged DU)"
ud="$(unmerged UD)"
aa="$(unmerged AA)"

count() { printf '%s\n' "$1" | grep -c . ; }

plural() {
    if [ "$1" -eq 1 ]; then echo "$1 $2"; else echo "$1 ${2}s"; fi
}
files() { plural "$1" file; }
blocks() { plural "$1" block; }

echo "## upstream \`$TAG\` → \`main\`"
echo
echo "Assembled by \`upstream-watch\`. **This branch carries conflict markers — do"
echo "not take it out of draft until they are resolved.** Full release notes:"
echo "<https://github.com/$UPSTREAM/releases/tag/$TAG>"
echo

# --- content conflicts, split by whether a human should look at them ----------

hand_total=0
hand_rows=""
lock_total=0
lock_rows=""
while read -r f; do
    [ -n "$f" ] || continue
    n="$(conflict_blocks "$f")"
    if is_lockfile "$f"; then
        lock_total=$((lock_total + n))
        lock_rows+="$n\t$f"$'\n'
    else
        hand_total=$((hand_total + n))
        hand_rows+="$n\t$f"$'\n'
    fi
done <<< "$uu"

# Worst file first: whoever opens this wants to know what the day looks like.
render_rows() { printf '%b' "$1" | grep -v '^$' | sort -rn \
    | awk -F'\t' '{ printf "- [ ] `%s` — %s\n", $2, $1 }'; }

if [ -n "$hand_rows" ]; then
    echo "### Resolve by hand — $(blocks "$hand_total")"
    echo
    render_rows "$hand_rows"
    echo
fi

if [ -n "$lock_rows" ]; then
    echo "**Regenerate, do not edit — $(blocks "$lock_total")**"
    echo
    render_rows "$lock_rows"
    echo
fi

if [ -z "$uu" ]; then
    echo "### No content conflicts"
    echo
    echo "The merge came out clean. Check the build, then take it out of draft."
    echo
fi

# --- deletions and collisions: the rest of the hand work ----------------------

if [ -n "$ud" ]; then
    echo "### Need a decision — $(files "$(count "$ud")")"
    echo
    echo "This fork modified these, upstream deleted them. Nothing automatic covers"
    echo "it: either accept the deletion and lose our changes, or keep the file with"
    echo "no upstream counterpart. Right now the file is **kept**."
    echo
    printf '%s\n' "$ud" | sed 's/^/- [ ] `/; s/$/`/'
    echo
fi

if [ -n "$aa" ]; then
    echo "### Name collisions — $(files "$(count "$aa")")"
    echo
    printf '%s\n' "$aa" | sed 's/^/- [ ] `/; s/$/`/'
    echo
fi

# --- how to pick this up locally ---------------------------------------------

# In a detached worktree `rev-parse --abbrev-ref` answers "HEAD"; the
# workflow knows the real name and passes it.
branch="${BRANCH:-$(git rev-parse --abbrev-ref HEAD)}"

echo "### Continue locally"
echo
echo '```sh'
echo "git fetch origin"
echo "git switch $branch"
echo
echo "# resolve the conflicts, then"
echo "git add -A && git commit"
echo "git push"
echo "gh pr ready"
echo '```'
