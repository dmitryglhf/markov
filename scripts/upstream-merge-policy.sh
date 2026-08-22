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
# Run with a merge in progress, before anything stages the index.
##############################################################################

set -euo pipefail

# comm below compares two sorted lists; both must use the same collation.
export LC_ALL=C

git rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1 \
    || { echo "no merge in progress" >&2; exit 1; }

base="$(git merge-base HEAD MERGE_HEAD)"

both="$(comm -12 \
    <(git diff --name-only "$base" HEAD | sort) \
    <(git diff --name-only "$base" MERGE_HEAD | sort))"

# $1 = attribute value to match, $2 = tree to restore the file from
apply() {
    [ -n "$both" ] || return 0

    printf '%s\n' "$both" \
        | git check-attr --stdin merge \
        | sed -n "s/: merge: $1\$//p" \
        | while IFS= read -r f; do
            # One side deleting the file is a tree conflict, not a content
            # one; no driver is called for it either. Leave it to the caller.
            git cat-file -e "$2:$f" 2>/dev/null || continue
            git checkout "$2" -- "$f"
            printf '  %s\n' "$f"
        done
}

echo "kept our version (merge=ours):"
apply ours HEAD
echo "took upstream's version (merge=theirs):"
apply theirs MERGE_HEAD
