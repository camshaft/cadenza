# PLAUSIBLE miscompile: a boolean op-membership helper called from `parse-cmp` corrupts an UNRELATED parse

**Reporter:** v-compiler-ml (2026-07-17, adding relational ops to the Cadenza-in-Cadenza parser).
**Status:** worked around (inlined the helper); the FEATURE shipped green. Mechanism UNEXPLAINED — needs a
trace to confirm it is a real miscompile vs. some subtler cause. Flagging as PLAUSIBLE, not confirmed.

## Observed

In `implementation/compiler-ml/src/parse-db.cdz`, `parse-cmp` decides whether the next token is a relational
operator. Two equivalent formulations:

**(A) helper — FAILED:**
```
def parse-cmp(...) = (match parse-expr(...) with | (lhs, j, t1) =>
  (let op = op-code(tok-at(ts, j)) in
   if is-cmp-op(op) then cmp-tail(ts, j, lhs, t1, op) else (lhs, j, t1)))
def is-cmp-op(op: Int64) =
  (if op == 60 then true else (if op == 61 then true else (if op == 62 then true
   else (if op == 63 then true else (if op == 64 then true else (if op == 65 then true else false))))))
```

**(B) inline range check — PASSES:**
```
def parse-cmp(...) = (match parse-expr(...) with | (lhs, j, t1) =>
  (let op = op-code(tok-at(ts, j)) in
   if (op >= 60) then (if (op <= 65) then cmp-tail(ts, j, lhs, t1, op) else (lhs, j, t1)) else (lhs, j, t1)))
```

With (A), the UNRELATED test `pd-deep-nesting` — parse `((1))`, expect one `NLit 1` node at the root —
FAILED its `is-lit(tree, root, 1)` assertion (root no longer the literal), while `node-count == 1` and
`root == 0` still held. Reproduced across THREE full-suite runs AND an isolated single-`@test` probe of
`is-lit(root,1)` (so NOT machine contention — it was a printed assertion failure, deterministic). Switching
ONLY to (B) → `pd-deep-nesting` passes, parse-db 27/27, full suite 1166/0.

## Why this is odd / needs a trace

`((1))` never reaches a relational operator, so `is-cmp-op` is only ever called with `op = -1` (the
`op-code` of `TRParen`) and returns `false` — identical to (B). Yet (A) corrupts the *result* of
`parse-tokens` for that input (the returned root id points at a non-literal / wrong node even though only one
node exists). That a pure boolean helper on `Int64` changes an unrelated parse result smells like a
codegen/monomorphisation aliasing bug, but the mechanism is unclear — hence PLAUSIBLE.

## Ask

If a compiler owner can diff the emitted core/wasm for (A) vs (B) on `parse-db.cdz` (or run the module under
a phase trace), it would confirm/deny a real miscompile. If confirmed, it is the same neighbourhood as the
`parse-if`→`parse-bool` hang I filed (`mlrepro-parse-if-cond-via-parse-bool-mutrec-hangs-compiler.md`) —
both are "adding a small function to the parse-db SCC misbehaves." Low urgency (workaround is clean), but the
"pure helper silently corrupts an unrelated result" shape is worth a look.

## PM triage (corpus-bugfix, 2026-07-20, trunk 995fa4134)
Does NOT reproduce STANDALONE (minimal is-cmp-op nested-if + tuple result computes correctly, 70/71) —
confirms it's module-scale-emergent in the full parse-db SCC, as reported. Its SIBLING (same neighborhood:
mlrepro-parse-if-cond-via-parse-bool-mutrec-hangs) is RESOLVED on trunk (parse-bool cluster + emit fix,
f813f5cd0-era; marked .RESOLVED earlier). So this MAY already be healed by that fix. ASKED v-compiler-ml to
un-inline the is-cmp-op helper (formulation A) + re-run pd-deep-nesting/parse-db on current trunk: green ->
mark RESOLVED + pin a helper-in-SCC case; still red -> route to v-inference as a confirmed scale-emergent
miscompile. Low urgency (clean workaround). Awaiting v-compiler-ml re-verify.
