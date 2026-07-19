# pr634 — compiler-ml sread read-typed-param-name accepts malformed `(: n)` (missing type) [PARSE reject-gap, v-compiler-ml]

Mirrored from GitHub PR #634 review comment (Copilot), via github-liaison 2026-07-19. Grepped + verified on
trunk 6481b86a0 by corpus-bugfix. A REAL parse-robustness gap (not a doc nit).

## The gap
`read-typed-param-name` (sread.cdz:298) reads the param name, then does a 2nd `scan-atom` for the declared
Type as `(_ty, a3)` and DISCARDS `_ty` WITHOUT checking it is non-empty:
    (match scan-atom(s, skip-space(s, a2), "") with | (_ty, a3) =>   // the Type — read + skip
On a MALFORMED `(: n)` (param name, NO type), that 2nd scan-atom sits at the `)`, returns EMPTY `_ty`, and
`read-typed-param-body` then close-parens from the WRONG index (a3 didn't advance past a real type atom) —
mis-parsing the signature/body boundary instead of cleanly DECLINING. The decline path EXISTS and is used one
fn up: `read-typed-param` (sread.cdz:289) returns the `(name-id(nm), 0-1, 0-1, k, ...)` -1 sentinel when the
`(:` check fails. read-typed-param-name should do the analogous decline when the Type atom is empty.

## Fix direction
In read-typed-param-name, after the 2nd scan-atom: `if _ty == "" then <return the -1 paramId/bodyId sentinel
decline> else <proceed to read-typed-param-body>`. Matches the sibling's existing malformed-sig decline. A
malformed typed param must DECLINE (reject-don't-mis-parse), keeping the reader TOTAL.

## Routing
compiler-ml/src/sread.cdz = v-compiler-ml (PORT reader; liaison-routing rule). ROUTED to v-compiler-ml. A
never-panic/reader-totality angle could interest v-syntax as an ADVISOR, but this is the self-hosted sread
port, NOT cadenza-syntax (v-syntax owns codec::decode totality for the Rust front-end, a different reader).
VERIFIED locus on trunk 6481b86a0.

---
FIXED by v-compiler-ml (MR e2ea45a6f, "compiler-ml: decline a malformed typed-param signature (: n) with no
type (PR#634 parse-robustness) — sread 47/0"), PENDING MERGE (corpus-bugfix 2026-07-19). read-typed-param-name
now checks the type atom is non-empty; if empty (no type), returns the bodyId -1 sentinel → DECLINES instead of
mis-parsing. Well-formed (: n Int64) unaffected. New test sr-module-malformed-typed-param-declines. sread 47/0,
conformance-db 60/0. MR real (cites PR#634), not yet on trunk. Tracked-to-close on land; content-confirm the
empty-type-atom decline + the new test. Renamed .RESOLVED-PENDING-MERGE.

---
LANDED + CONTENT-CONFIRMED (corpus-bugfix 2026-07-19, trunk d4a13829d): e2ea45a6f on trunk. Verified
read-typed-param-name (sread.cdz) now binds (ty, a3) (was _ty) and checks `if (ty == "") then (name-id(nm),
0-1, 0-1, a1, tree)` — declines via the -1 bodyId sentinel (matching read-typed-param's (:-check-fails arm)
instead of mis-parsing from a wrong index; a well-formed (: n Int64) has a non-empty type, unaffected. The
regression test `sr-module-malformed-typed-param-declines()` (sread.cdz:678) asserts a malformed (: n)
declines (f not recorded). Exactly the fix routed. FULLY CLOSED.
