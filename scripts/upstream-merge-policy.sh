#!/usr/bin/env bash
##############################################################################
# Apply the per-path merge policy declared in .gitattributes.
#
# Git has no built-in `ours`/`theirs` merge driver. The names in
# .gitattributes mean nothing until a command is registered under merge.* in
# .git/config, which is unversioned, has to be repeated in every clone and on
# every runner, and — the real problem — decays silently: forget it and the
# attributes turn back into an ordinary three-way merge with no warning.
#
# So the policy is applied here instead, after the merge, with plain
# checkouts. .gitattributes stays the single versioned source of truth; there
# is nothing to install and nothing to keep in sync.
#
# This mirrors driver semantics rather than improving on them: a driver is
# only invoked when BOTH sides changed a file, so a file only upstream touched
# keeps upstream's version. Hence the intersection below.
#
# Direction matters. `ours` in .gitattributes means this fork, which is not
# always the side git calls HEAD:
#
#     git merge <upstream-tag>     while on main    -> fork is HEAD    (default)
#     git merge origin/main        while on the     -> fork is MERGE_HEAD
#                                     upstream branch
#
# Pass the fork's side when it is not HEAD. Getting this backwards silently
# hands the frozen directories to upstream, so the script prints which side it
# took to be ours.
#
# Run with a merge in progress, before anything stages the index.
#
#     scripts/upstream-merge-policy.sh [HEAD|MERGE_HEAD]
##############################################################################

set -euo pipefail

# comm below compares two sorted lists; both must use the same collation.
export LC_ALL=C

FORK="${1:-HEAD}"
case "$FORK" in
    HEAD)       UPSTREAM=MERGE_HEAD ;;
    MERGE_HEAD) UPSTREAM=HEAD ;;
    *) echo "usage: $0 [HEAD|MERGE_HEAD]" >&2; exit 2 ;;
esac

git rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1 \
    || { echo "no merge in progress" >&2; exit 1; }

base="$(git merge-base HEAD MERGE_HEAD)"

both="$(comm -12 \
    <(git diff --name-only "$base" "$FORK" | sort) \
    <(git diff --name-only "$base" "$UPSTREAM" | sort))"

# $1 = attribute value to match, $2 = tree to restore the file from
apply() {
    [ -n "$both" ] || return 0

    printf '%s\n' "$both" \
        | git check-attr --stdin merge \
        | sed -n "s/: merge: $1\$//p" \
        | while IFS= read -r f; do
            # One side deleting the file is a tree conflict, not a content
            # one; no driver is called for it either, so it is left below.
            git cat-file -e "$2:$f" 2>/dev/null || continue
            git checkout "$2" -- "$f"
            printf '  %s\n' "$f"
        done
}

echo "fork side: $FORK, upstream side: $UPSTREAM"
echo "kept our version (merge=ours):"
apply ours "$FORK"
echo "took upstream's version (merge=theirs):"
apply theirs "$UPSTREAM"

# A file this fork deleted that upstream still edits is a tree conflict, and
# no attribute is consulted for it. The fork's answer is always the same:
# stay deleted. git labels the class from HEAD's point of view, so which one
# it is depends on the direction of the merge.
[ "$FORK" = HEAD ] && CLASS=DU || CLASS=UD
echo "kept deleted:"
git status --porcelain | awk -v c="$CLASS" '$1 == c { print $2 }' \
    | while IFS= read -r f; do
        git rm -q -f -- "$f"
        printf '  %s\n' "$f"
    done
