# PR #474 (merged, batch 103) — notebook parseWidgets.ts: IDENT_RE too loose + no quote-escape handling

Mirrored from Copilot inline review on merged PR #474 (3 clustered comments), all in
`guide/src/notebook/parseWidgets.ts`. Confirmed on trunk. Owner: **v-notebook**.

## 1. IDENT_RE accepts invalid Cadenza identifiers (comment 3595398102, line 40)
> `IDENT_RE` currently accepts `.` and also permits kebab `-` in positions Cadenza identifiers
> cannot have (e.g. trailing `-` or `a--b`). Since `bindingFor()` emits `def ${widget.name}`, a
> widget named with `.`/`--`/trailing-`-` produces invalid Cadenza.

Trunk: `const IDENT_RE = /^[A-Za-z_][A-Za-z0-9_.-]*$/;` (line 40) — the char class `[A-Za-z0-9_.-]`
does allow `.`, `--`, and trailing `-`. `bindingFor` (line 179) does emit `def ${...}`. Real:
an invalid widget name flows into emitted Cadenza source.

## 2. splitArgs mishandles escaped quotes (comment 3595398162, line 61)
> `splitArgs` toggles `inStr` on every `"` and does not account for backslash-escaped quotes inside
> strings. A widget line like `dropdown("a \"quoted\" option", "b")` will be split incorrectly.

Trunk line 51: `if (ch === '"') inStr = !inStr;` — no `\` lookbehind. Real.

## 3. asString does not unescape (comment 3595398190, line 80)
> `asString` strips surrounding quotes but does not unescape `\\` / `\"`. Users cannot express
> dropdown/text defaults containing quotes.

Trunk line 77 `asString` strips quotes only. Real (same escaping gap as #2).

## Suggested fix
Tighten `IDENT_RE` to reject trailing/double `-` and `.` (Cadenza ident rules), and give the widget
arg-lexer real string handling (escape-aware split + unescape) so quoted defaults round-trip. These
three are one coherent DSL-lexer hardening pass.

PR: https://github.com/camshaft/cadenza/pull/474
