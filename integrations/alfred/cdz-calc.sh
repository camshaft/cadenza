#!/usr/bin/env bash
# Alfred Script Filter for the Cadenza calculator.
#
# Alfred runs this on every keystroke of the `=` keyword, passing the typed query as $1 (or as the
# `{query}` argument). It shells to `cdz-calc --once --plain` and emits ONE Alfred item (JSON) whose
# title is the result — so the user sees the value live as they type, and Enter copies it (via the
# item's `arg`, wired to a Copy-to-Clipboard action in the workflow).
#
# Config via workflow environment variables (set in Alfred's workflow UI, all optional):
#   CDZ_CALC   — path to the cdz-calc binary            (default: `cdz-calc` on $PATH)
#   CADENZA_STORE — the runtime store dir               (default: cdz-calc's built-in repo lookup)
#   CDZ_CALC_FLAGS — extra flags, e.g. `--sexpr` or `--no-exact` (default: none)
#
# Output is Alfred's Script Filter JSON (https://www.alfredapp.com/help/workflows/inputs/script-filter/).

set -uo pipefail

query="${1:-}"
bin="${CDZ_CALC:-cdz-calc}"

# JSON-escape a string for embedding in the Alfred output (quotes, backslashes, control chars).
json_escape() {
  # Use python3 (present on macOS) for correct escaping of any character.
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

emit_item() {
  # $1 = title, $2 = subtitle, $3 = arg (what Enter copies), $4 = valid ("true"/"false")
  local title subtitle arg
  title=$(json_escape "$1")
  subtitle=$(json_escape "$2")
  arg=$(json_escape "$3")
  printf '{"items":[{"uid":"cdz-calc","type":"default","title":%s,"subtitle":%s,"arg":%s,"valid":%s,"text":{"copy":%s,"largetype":%s}}]}\n' \
    "$title" "$subtitle" "$arg" "$4" "$arg" "$title"
}

# An empty query: prompt, not an error.
if [[ -z "${query// }" ]]; then
  emit_item "…" "Type an expression — e.g. 1 / 3, 0.1 + 0.2, 1000000 * 1000000" "" "false"
  exit 0
fi

# Evaluate. stdout = the value, stderr = an error message, exit code says which.
if result=$("$bin" --plain ${CDZ_CALC_FLAGS:-} --once "$query" 2>/tmp/cdz-calc-err); then
  emit_item "$result" "= $query   ↵ to copy" "$result" "true"
else
  err=$(cat /tmp/cdz-calc-err 2>/dev/null)
  emit_item "$query" "${err:-not a valid expression}" "" "false"
fi
