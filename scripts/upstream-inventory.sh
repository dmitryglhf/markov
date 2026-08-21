#!/usr/bin/env bash
# Renders the body of the upstream-merge pull request.
#
# Run it while `git merge <tag>` is still unresolved: the conflict classes come
# from the index, which is wiped the moment the merge is committed. Prints
# markdown to stdout and nothing else, so the caller can redirect it into a file.
#
#     git merge --no-ff v1.47.0 || scripts/upstream-inventory.sh v1.47.0 > body.md

set -uo pipefail

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

# "1 файл", "2 файла", "5 файлов"
plural() {
    n=$1; one=$2; few=$3; many=$4
    case $((n % 100)) in
        1[1-9]) echo "$n $many"; return ;;
    esac
    case $((n % 10)) in
        1) echo "$n $one" ;;
        2|3|4) echo "$n $few" ;;
        *) echo "$n $many" ;;
    esac
}
files() { plural "$1" файл файла файлов; }
blocks() { plural "$1" блок блока блоков; }

echo "## upstream \`$TAG\` → \`main\`"
echo
echo "Ветку собрал \`upstream-watch\`. **Внутри маркеры конфликтов — черновик снимать"
echo "только после разрешения.** Полные заметки к релизу:"
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
    echo "### Разрешить руками — $(blocks "$hand_total")"
    echo
    render_rows "$hand_rows"
    echo
fi

if [ -n "$lock_rows" ]; then
    echo "### Перегенерировать, не править — $(blocks "$lock_total")"
    echo
    render_rows "$lock_rows"
    echo
fi

if [ -z "$uu" ]; then
    echo "### Содержательных конфликтов нет"
    echo
    echo "Слияние прошло чисто. Проверить сборку и снимать черновик."
    echo
fi

# --- deletions, both directions ----------------------------------------------

if [ -n "$du" ]; then
    echo "### Наши удаления сохранены — $(files "$(count "$du")")"
    echo
    echo "Апстрим их правил, мы их удалили; выбрано наше. Проверить, что среди них"
    echo "нет ничего, что стоило бы вернуть."
    echo
    printf '%s\n' "$du" | sed 's/^/- `/; s/$/`/'
    echo
fi

if [ -n "$ud" ]; then
    echo "### Требуют решения — $(files "$(count "$ud")")"
    echo
    echo "Мы правили, апстрим удалил. Автоматика тут не решает: либо принять"
    echo "удаление и потерять наши правки, либо оставить файл вне апстрима."
    echo "Сейчас файл **оставлен**."
    echo
    printf '%s\n' "$ud" | sed 's/^/- [ ] `/; s/$/`/'
    echo
fi

if [ -n "$aa" ]; then
    echo "### Столкновение имён — $(files "$(count "$aa")")"
    echo
    printf '%s\n' "$aa" | sed 's/^/- [ ] `/; s/$/`/'
    echo
fi

# --- what the merge drivers threw away, which the diff cannot show ------------

ours_dropped="$(
    comm -12 \
        <(git diff --name-only "$BASE" HEAD | git check-attr --stdin merge \
            | sed -n 's/: merge: ours$//p' | sort) \
        <(git diff --name-only "$BASE" "$TAG" | sort)
)"
if [ -n "$ours_dropped" ]; then
    n="$(printf '%s\n' "$ours_dropped" | wc -l | tr -d ' ')"
    echo "### Апстримные правки отброшены драйвером — $(files "$n")"
    echo
    echo "\`merge=ours\` по \`.gitattributes\`. Это не конфликты: изменения апстрима"
    echo "в этих файлах выброшены целиком, включая непересекающиеся, и **в диффе"
    echo "их не видно**."
    echo
    echo '<details><summary>показать</summary>'
    echo
    printf '%s\n' "$ours_dropped" | sed 's/^/- `/; s/$/`/'
    echo
    echo '</details>'
    echo
fi

# --- upstream work that landed in code we have forked ------------------------

ours="$(git diff --name-only --diff-filter=M "$BASE" HEAD | tr '\n' ' ')"
if [ -n "$ours" ]; then
    all="$(git rev-list --count "$BASE..$TAG")"
    # Unquoted on purpose: one pathspec per file. No path in this repo has a space.
    mine="$(git log --oneline --no-decorate "$BASE..$TAG" -- $ours 2>/dev/null)"
    n="$(count "$mine")"
    echo "### Апстрим в наших файлах — $(plural "$n" коммит коммита коммитов) из $all"
    echo
    echo '<details><summary>показать</summary>'
    echo
    echo '```'
    printf '%s\n' "$mine"
    echo '```'
    echo
    echo '</details>'
    echo
fi

# --- how to pick this up locally ---------------------------------------------

# In a detached worktree `rev-parse --abbrev-ref` answers "HEAD"; the
# workflow knows the real name and passes it.
branch="${BRANCH:-$(git rev-parse --abbrev-ref HEAD)}"
echo "### Продолжить локально"
echo
echo '```sh'
echo "git fetch github && git checkout $branch"
echo "just markov-setup-merge   # драйверы живут в .git/config, у каждого свои"
echo '```'
