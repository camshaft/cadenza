# PR review comments — mirrored from GitHub PR #416 (Copilot inline) — GUIDE scalar-format cluster

- **PR:** #416 "fleet: forty-first batch (guide scalar-format, …)" (OPEN at triage; files on trunk)
- **Files:** `guide/src/runner/scalarFormat.ts:10/25`, `guide/src/components/useCadenzaEditor.ts:135`, `guide/scripts/check-examples.mjs:148`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591572573, 3591572628, 3591572666, 3591572702
- **Links:** https://github.com/camshaft/cadenza/pull/416#discussion_r3591572573 (+ r3591572628, r3591572666, r3591572702)

## Comments (verbatim, condensed)
> [scalarFormat.ts:10] Comment says "match the type HEAD" but `isFloatType` matches `/\bFloat(32|64)\b/` ANYWHERE in the rendered type string. Anchor the regex to the head or fix the comment.
> [scalarFormat.ts:25] `formatScalarByType` checks `value.trim()` but appends `.0` to the *untrimmed* `value`; leading/trailing whitespace produces odd output (e.g. `" 5.0"`).
> [useCadenzaEditor.ts:135] `exportTypes(program, …)` is awaited UNCONDITIONALLY though the `.0` fix is only needed for integer-looking scalar renders — extra work on the common path.
> [check-examples.mjs:148] `runComponent` always calls `export_types(program, surface)` when program is provided, even though `.0` restoration is only relevant for integer-looking scalar renders — extra parsing/query work.

## Liaison triage — CONFIRMED (spot-check scalarFormat.ts:25)
Confirmed: `formatScalarByType` does `/^-?\d+$/.test(value.trim())` but returns `` `${value}.0` `` on the
UNTRIMMED value, so `" 5"` → `" 5.0"` (whitespace preserved) — a real formatting bug. The other three:
`isFloatType`'s unanchored regex (comment/behavior mismatch — could match a Float in a compound type
string), and TWO perf points (exportTypes/export_types awaited unconditionally when the `.0` fix is only
needed for integer-looking renders). All guide territory (v-guide), one coherent cluster around the new
scalar-`.0`-restoration feature. FIX: trim before appending; anchor/fix the isFloatType regex+comment;
gate the export-types call on an integer-looking render. Fix on `trunk`. Quotes + links in queue file.

<!-- RESOLVED 2026-07-16 (trunk@b706d3b76, v-guide-infra): LANDED + verified by file content on trunk. -->
