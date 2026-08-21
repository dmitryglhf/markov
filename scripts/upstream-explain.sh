#!/usr/bin/env bash
# Appends a short prose section explaining what the merge conflicts are about.
#
# Feeds the model the conflict hunks only — never the diff. Upstream pours
# +22k lines into crates/goose alone over a release window, and none of it helps
# answer the one question a human has here: why do these two sides disagree.
#
# Prints nothing at all unless the whole thing succeeds, so a bad key or a dead
# model leaves the deterministic part of the PR body untouched.

set -uo pipefail

MODEL="${OPENROUTER_MODEL:-x-ai/grok-4.6}"
BUDGET=60000   # characters of hunks; the rest is dropped with a note

: "${OPENROUTER_API_KEY:?OPENROUTER_API_KEY не задан}"

# Sides get labelled here rather than left as raw markers. Measured on the real
# 1.46 conflicts: handed the markers, the model confidently swapped who wanted
# what in two cases out of three, always where one side was empty. Handed
# "[форк добавил] / [апстрим добавил]", it got all ten blocks right.
relabel() {
    awk -v FN="$1" '
        /^<<<<<<< /  { n++; side=1; ours=""; theirs=""; next }
        /^\|\|\|\|\|\|\| / { side=3; next }          # diff3 base section, ignored
        /^=======$/  { if (side) { side=2; next } }
        /^>>>>>>> /  {
            side=0
            printf "--- %s, блок %d ---\n[форк добавил]\n%s[апстрим добавил]\n%s\n",
                   FN, n, (ours=="" ? "(ничего)\n" : ours),
                           (theirs=="" ? "(ничего)" : theirs)
            next
        }
        side==1 { ours = ours $0 "\n"; next }
        side==2 { theirs = theirs $0 "\n"; next }
    ' < "$2"
}

hunks=""
dropped=0
while read -r f; do
    [ -n "$f" ] || continue
    case "$(basename -- "$f")" in
        Cargo.lock|pnpm-lock.yaml|package-lock.json|yarn.lock|flake.lock) continue ;;
    esac
    piece="$(relabel "$f" "$f" 2>/dev/null)"
    [ -n "$piece" ] || continue
    piece="$piece
"
    if [ $(( ${#hunks} + ${#piece} )) -gt "$BUDGET" ]; then
        dropped=$((dropped + 1))
        continue
    fi
    hunks+="$piece"
done < <(git status --porcelain | awk '$1 == "UU" { print $2 }')

[ -n "$hunks" ] || exit 0

read -r -d '' PROMPT <<'PROMPT_END' || true
Ниже конфликты git из слияния форка с апстримом. Стороны уже подписаны.

На каждый блок — одна строка "- `путь` (блок N) — ...": чего хочет каждая
сторона и почему столкнулись. По-русски, только список, без вступления и
выводов. Если намерение из блока не читается, так и напиши.
PROMPT_END

# Ризонинг здесь только мешает — нужен список, а не размышление. Но единой
# политики нет: nvidia/nemotron-3.5-lightning с включённым ризонингом съедает
# 7000 токенов и возвращает content: null, а x-ai/grok-4.6 на попытку его
# выключить отвечает 400 "Reasoning is mandatory for this endpoint". Модель
# задаётся переменной репозитория, так что выбирать приходится на месте:
# просим выключить, а на отказ соглашаемся.
payload="$(jq -n --arg m "$MODEL" --arg p "$PROMPT" --arg h "$hunks" \
    '{model: $m, max_tokens: 6000, temperature: 0,
      reasoning: {enabled: false},
      messages: [{role: "user", content: ($p + "\n\n" + $h)}]}')"

ask() {
    # 2>/dev/null: первая попытка заведомо может вернуть 400 на reasoning,
    # и сообщение curl в stderr читалось бы как настоящая ошибка. Текст
    # ошибки всё равно приходит телом и разбирается ниже.
    curl -sS --fail-with-body --max-time 600 2>/dev/null \
        https://openrouter.ai/api/v1/chat/completions \
        -H "Authorization: Bearer $OPENROUTER_API_KEY" \
        -H "Content-Type: application/json" \
        -d "$1"
}

resp="$(ask "$payload")" || {
    if printf '%s' "$resp" | grep -qi 'reasoning'; then
        resp="$(ask "$(printf '%s' "$payload" | jq 'del(.reasoning)')")" \
            || { echo "запрос к OpenRouter не удался: $resp" >&2; exit 1; }
    else
        echo "запрос к OpenRouter не удался: $resp" >&2; exit 1
    fi
}

text="$(printf '%s' "$resp" | jq -r '.choices[0].message.content // empty')"
[ -n "$text" ] || { echo "пустой ответ: $resp" >&2; exit 1; }
cost="$(printf '%s' "$resp" | jq -r '.usage.cost // empty')"

# Only now, when everything worked, touch stdout.
echo
echo "### О чём конфликты"
echo
echo "> Сгенерировано моделью \`$MODEL\`${cost:+ за \$$cost} по конфликтным блокам."
echo "> Подсказка, не документ: числа и списки файлов выше проверяемы, этот"
echo "> раздел — нет."
echo
printf '%s\n' "$text"
if [ "$dropped" -gt 0 ]; then
    echo
    echo "_Ещё $dropped файлов не поместились в бюджет и модели не показывались._"
fi
