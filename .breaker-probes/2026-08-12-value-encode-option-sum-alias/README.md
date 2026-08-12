# 2026-08-12 value-encode Option-sum/Option-Bytes descriptor alias (tick 1351)
# FINDING #22 tracking (concierge-routed, v-effects diagnosed, v-rust-backend owns fix)

- `vse1.sexp` — THE BUG (sexp twin of the queue's ML repro): a record result with
  SIBLING fields payload=Some(P.B{x="hi"}) (P = sum A(Bytes)|B(record)) and
  correlation=None:Option<Bytes>. wasm value-encodes payload as `(Some b"")` —
  EMPTY BYTES-LEAF instead of the sum value-form = silent data loss. rust +
  rust-async encode `(Some (B (record (= x "hi"))))` correctly. FAILS wasm today;
  PIN ON FIX.
- `vse2.sexp` — control: Option-sum field alone → intact on wasm (PASS).
- `vse3.sexp` — control: two Option-Bytes fields alone → intact on wasm (PASS).
Matches the routed diagnosis: either alone correct, together broken. Same
descriptor-alias family as #18/#21 but in the VALUE-ENCODE walker, not the emitter.

Render-form note for pin time: wasm renders record results WITH a lowercase
`(record (field ty))` type annotation; rust renders WITHOUT. vse2/vse3 pass wasm
with the annotated expectation; vse1's rust pass used the plain form. Compose the
post-fix pin against all three renders (check how 05-compound's annotated record
outputs pass the rust baseline — likely normalized comparison).

## Tick 1352 boundary map — ROOT SHAPE IDENTIFIED
The walker reuses the FIRST (field-sort-order) Option field's payload descriptor
for EVERY LATER Option sibling in the record:
| probe | shape | wasm |
|---|---|---|
| vse1 | correlation(Option Bytes) sorts BEFORE payload(Option sum) | FAIL: sum → `Some b""` (bytes descriptor) |
| vse5 | count(Option Int64) before payload(Option sum) | FAIL: sum → `Some 1` (INT descriptor!) |
| vse6 | sum WITHOUT Bytes arm, Option-Bytes sibling | FAIL: same clobber (sum arms irrelevant) |
| vse7 | TWO Option-sums first(P)/second(Q) | FAIL: second → P's descriptor over Q's value: `Some (B (record (= x "")))` |
| vse9 | names reversed so the SUM sorts FIRST | PASS — first field always intact |
| vse8 | BARE sum + Option-Bytes sibling | PASS — only Option-wrapped fields share |
| vse4 | the pair built in an ARM, crosses resume | FAIL — not return-position-specific |
So: not Bytes-specific, not sum-specific — the FIRST Option descriptor is memoized
and applied to all later Option fields. Fix-verify plan: vse1/4/5/6/7 flip to pass,
vse2/3/8/9 stay pass. Promote vse1+vse7+controls on land.

## Tick 1354 additions (pre-fix binary, fix MR 2b6c82f9b queued)
- `vse10.sexp` — RESULT family: (Result Int64 String) + (Result Int64 Bytes) siblings →
  second's Err renders "no" (STRING descriptor) instead of b"no". FAILS pre-fix →
  flip-on-land. Confirms the family generalization (any generic decl, not just Option).
- `vse11.sexp` — LIST family: (List Int64) + (List String) siblings → PASS even
  pre-fix (lists don't route through the sums memo). Control.
Fix-verify set on land: vse1/4/5/6/7/10 flip to pass; vse2/3/8/9/11 stay pass.

## Tick 1356 — FIX VERIFIED, FINDING #22 CLOSED
Fix landed origin 94a289481 (re-sha of 2b6c82f9b: descriptor memo keyed by full
instantiation at Ty::Sum + Ty::Nominal). Fresh worktree-local cdz + store from the
landed sha: ALL 11 vse probes PASS ×3 (wasm/rust/rust-async). vse1's expectation
re-annotated to the wasm render form (the gate accepts it on rust too — comparison
normalizes). Flip set confirmed: vse1/4/5/6/7/10 red→green; controls stayed green.
PROMOTION SET (next batch): vse1 (minimal) + vse7 (two-sums directional) + vse10
(Result family) + vse9 (sort-order control) + vse11 (List control).
