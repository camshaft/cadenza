# DESIGN — nested list/collection patterns in the cadenza sum-match re-emit

Owner: v-cadenza-backend. Status: DESIGN (banked for a fresh-context implementation, following the M4a
process — see `DESIGN-matchsum-nested-pattern-whole-slot.md`). Scope: the `--target cadenza` re-emit of a
`match` whose decision tree tests a LIST (or Bytes/Map) sub-value NESTED inside the SUM-match tree — a
list-length probe or a list-element/rest read reached through a sum arm, NOT the direct `Core::MatchList`
path (which already works).

## 1. The symptom

Two `spec/semantics/20-structural-editing.sexp` cases decline on `--target cadenza` (both PASS on wasm/rust):

- **"a NONZERO BigInt literal probe in a recursive quasiquote-pattern simp matches its own constructor"**
  — `simp`'s quasiquote arm `` `(* ,x 1) `` matches an `Ast.List` of a fixed shape. The decision tree emits
  a **`ListLen` LitTest probe** on the list slot. Declines:
  `CDZ0900 "the Cadenza backend reconstructs a literal-at-slot test only for an Int / Bool / Str / Char
  probe (a Bytes / ListLen / MapHasKeys slot probe is not supported)"`.

- **"a mutually-recursive fold matching a rebuilt list with a payload binder builds and computes"** —
  `fold`/`fold-list` walk an `Ast.List`, matching `#list(h (.. t))` (a list-rest read of `t`) and
  `#list((Ast.Int a))` (a nested-sum read of `a` inside a list element), all NESTED inside sum-match arms.
  Declines:
  `CDZ0900 "the Cadenza backend does not support lowering a nested match sub-pattern with a non-tuple/record
  (sum / list-rest) step"`.

Both are the SAME frontier: a collection sub-pattern appears inside the SUM-match decision tree, where the
reconstruction (`emit_switch_tree` / `build_arm_pat` / the `Core::SumPayload` read resolution) only handles
`Elem` (tuple/record projection) steps and scalar LitTest probes — not `RestFrom`/list-`Payload` reads or
`ListLen`/`Bytes`/`MapHasKeys` probes.

## 2. What already works (the machinery to reuse)

- `emit_match_list` (~mod.rs:4638) reconstructs a DIRECT `Core::MatchList` as a surface
  `(match <scrut> (<list-pattern> <body>)…)`: `ListArmCond::LenEq(n)` → `(list b0 … b_{n-1})`,
  `LenGe(lead)` → `(list b0 … b_{lead-1} .. rest)`, `Any` → `_`. Leading element binders register at
  `[Elem(i)]`, the rest binder at `[RestFrom(lead)]` — the exact `Core::SumPayload` keys the body reads.
  BUT it emits only PLAIN element binders: a NESTED element sub-pattern (`(list (Mk x) ..)`) registers at a
  deeper path this slice does not handle and DECLINES.
- The scalar LitTest reconstruction in `emit_switch_tree` (~mod.rs:3813) emits an Int/Bool/Str/Char literal
  IN the surface pattern via `lit_choices`, then the fall-through `els` unrefined.
- `build_arm_pat` (~mod.rs:3319 wrapper / `build_arm_pat_inner`) reconstructs a sum arm's flattened pattern,
  descending `Elem` steps into tuple/record projections; M4a's B1/B2 (whole-slot binder + `let`
  reconstruction) live here.

The gap is that NONE of the sum-match reconstruction paths reach into the list machinery for a nested list
probe/read.

## 3. The two decline sites (fix points)

All in `implementation/seed/crates/rcdzc/src/backend/cadenza/mod.rs`:

- **Site A — `Core::SumPayload` nested read, ~mod.rs:2325-2331.** The nested-compound read walk descends only
  `Elem` steps (tuple index / record field); a `Payload` (nested sum) or `RestFrom` (list rest) step declines.
  This is what the fold case's `t` (`RestFrom`) and `a` (`… Payload` inside a list element) hit.
- **Site B — `emit_switch_tree` LitTest arm, ~mod.rs:3827.** A `Probe::ListLen`/`Bytes`/`MapHasKeys` probe is
  not reconstructed into a surface pattern (only scalar Int/Bool/Str/Char). This is what the quasiquote case's
  `ListLen` probe hits.

## 4. Approach (sketch — to be refined by the implementer)

The unifying idea: when the sum-match tree tests/reads through a LIST slot, emit a surface LIST PATTERN
(`(list …)` / `(list … .. rest)`) at that slot and reuse the `emit_match_list` element/rest binder
registration (`[Elem(i)]` / `[RestFrom(lead)]`), recursing element sub-patterns through `build_arm_pat`.

- **Site B (ListLen probe):** extend `lit_choices` (or a parallel `list_choices`) so a `ListLen { len,
  at_least }` slot emits a `(list _0 … _{len-1} [.. rest])` pattern whose element slots are fresh binders
  the deeper tree/body reads (at `[Elem(i)]`/`[RestFrom(len)]`), mirroring `emit_match_list`. Start with
  `at_least=false` (fixed arity) as the first sub-slice; add the rest form second.
- **Site A (RestFrom / list-`Payload` read):** teach the nested-read walk to cross a `RestFrom(k)` step
  (bind/read the tail sublist) and a list-element `Payload` step (a nested sum inside a list element →
  recurse the sum reconstruction). The rest read `t` is the simpler, independently-landable first sub-slice.

**Idempotence is NOT required** (the cadenza gate is hop1→hop2→run→compare VALUE, no byte-idempotence check —
verified in `run_program_cadenza`, xtask/main.rs), so a value-equivalent list-pattern re-emit suffices.

**Respect the existing fences.** `emit_match_list` carries the #5472 fence (a list match over a scrutinee
with a RUNTIME-valued map element does NOT round-trip → decline). Any nested-list re-emit must keep that
fence (and the mfp1/mfp2 class): re-emit only shapes that re-lower identically, else DECLINE
(reject-don't-miscompile) — never emit a `program1` the compiler cannot re-lower (a corpus-cadenza RED, not
a skip). This is why the frontier warrants a design: the failure mode of a rushed change is a RED, not a
clean decline.

## 5. Migration / corpus impact

- Target cases: the two `20-structural-editing` cadenza todos (§1). Land each sub-slice with the case(s) it
  flips; a sub-slice that only partially covers a case leaves it todo (no regression).
- Verify NO regression on the DIRECT list-match cases (`emit_match_list` is reused, not replaced) and the
  sum-match corpus (05/13/17/20/26) via `cargo xtask gate <file> --target cadenza` + `--show-declines`
  deltas. wasm/rust untouched (the change is entirely in the cadenza backend).
- Each sub-slice is independently landable and additive (turns a decline into a pass; a still-unhandled
  shape keeps declining).

## 6. Fix-point index

All in `backend/cadenza/mod.rs`:
- `emit_match_list` (~4638) — the DIRECT list-match reconstruction to reuse (element `[Elem(i)]` / rest
  `[RestFrom(lead)]` binder registration; `emit_list_elem_binder`).
- The `Core::SumPayload` nested-read walk (~2310-2331) — Site A; add `RestFrom` + list-`Payload` step
  handling to the `for step in path` loop (today only `Elem` on Tuple/Record).
- `emit_switch_tree`'s `SumCont::LitTest` arm (~3807-3836) — Site B; add a `Probe::ListLen` case emitting a
  list pattern (fixed-arity first).
- `build_arm_pat` / `build_arm_pat_inner` (~3319) — where a list slot's sub-pattern integrates (recurse
  element sub-patterns; M4a's whole-slot reconstruction is the adjacent precedent).
- The #5472 fence in `emit_match_list` (~4659) — the round-trip-break guard the nested path must preserve.

No code change — design only.
