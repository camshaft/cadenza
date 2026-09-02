# DESIGN — MatchSum nested-pattern whole-slot read (the M4a completeness gap)

Owner: v-cadenza-backend. Status: DESIGN (banked for a fresh-context implementation, per operator ruling
2026-09-02: "have v-cadenza-backend do a write up for the big redesign and restart it with a fresh context").
Scope: the `--target cadenza` backend re-emit of a nested-sum `match` whose arm body reads a WHOLE payload
slot that the arm ALSO destructures or wildcards. Currently a completeness gap → CDZ0101 on the round-trip
(a cadenza-target TODO; NOT a silent miscompile — always validate-caught).

## 1. The symptom

`spec/semantics/20-structural-editing.sexp` case "a rewrite-then-eval pipeline over a runtime tree
preserves meaning through the rewrite" declines on `--target cadenza` (the ONLY CDZ0101 in that file).
Minimal program:

```
(do
  (type Exp (Lit Int64) (Add Exp Exp))
  (def (simplify (: e Exp))
    (match e
      ((Add (Lit 0) r) (simplify r))
      ((Add l r) (Add (simplify l) (simplify r)))
      ((Lit n) (Lit n))))
  (def (eval-exp (: e Exp)) (match e ((Lit n) n) ((Add l r) (+ (eval-exp l) (eval-exp r)))))
  (def (build (: a Int64)) (Add (Lit 0) (Add (Lit a) (Lit 2))))
  (def (main (: a Int64)) (eval-exp (simplify (build a))))
  (export main))
```

`cdz compile … -t cadenza` SUCCEEDS (emits a cadenza AST), but that AST fails to RECOMPILE:
`error [CDZ0101]: unbound name '_cdz_m0'`. The emitted `simplify` is:

```
(match e
  ((Add (Lit 0) _cdz_m2) (simplify _cdz_m2))
  ((Add (Lit _cdz_m3) _cdz_m4) (: (Add (simplify _cdz_m0) (simplify _cdz_m4)) Exp))   ; _cdz_m0 UNBOUND
  ((Add _ _cdz_m5)          (: (Add (simplify _cdz_m0) (simplify _cdz_m5)) Exp))       ; _cdz_m0 UNBOUND
  ((Lit _cdz_m6) (: (Lit _cdz_m6) Exp)))
```

The `(Add l r)` body reads `l` = the WHOLE first Add payload. In the arms where that slot is destructured
(`(Lit _cdz_m3)`) or wildcarded (`_`), `l` is never bound, yet the body references `_cdz_m0` for it.

## 2. Root cause

The decision tree for `simplify` roots a `Switch` on `Exp`'s OWN discriminant (path `[]`), so
`emit_match_sum` takes the FLAT-arm loop (`mod.rs`: `SumCont::Switch { path, arms } if path.is_empty()`
→ `for arm in arms`). For the `Add` variant (arity 2), the flat loop:

1. MINTS whole-slot binders `_cdz_m0` (slot 0) and `_cdz_m1` (slot 1) and REGISTERS them in the shared
   `env.payloads` under `(scrutinee, [Payload, Elem(0)])` / `(…, [Payload, Elem(1)])`
   (`emit_match_sum`, the `for slot in 0..arity` block, ~mod.rs:4075-4088). These are the names the
   `(Add l r)` body's `Core::SumPayload` reads resolve to (exact-lookup at the `Core::SumPayload` arm,
   ~mod.rs:2204).
2. Because the `Add` arm's cont is a MULTI-payload nested `Switch` (on slot 0, `[Payload, Elem(0)]`, arity
   ≠ 1), it does NOT match the single-payload `emit_nested_switch_chain` arm; it falls to the general
   deep-tree branch **`SumCont::Switch { .. }` at ~mod.rs:4246**, which sets `choices[[]] = Some(Add-disc)`
   and calls `emit_switch_tree`.
3. `emit_switch_tree` + `build_arm_pat` FLATTEN the decision tree into surface arms with NESTED patterns:
   slot 0 becomes `(Lit 0)` / `(Lit _cdz_m3)` / `_`. `build_arm_pat` registers only the LEAF binders it
   emits (the inner `_cdz_m3`, the sibling `_cdz_m4`/`_cdz_m5`); it never emits a pattern binder for the
   WHOLE slot `[Payload, Elem(0)]`.

So `_cdz_m0` (`[Payload, Elem(0)]`) is REGISTERED (step 1) but NOT EMITTED in the flattened arms (step 3) —
a register-vs-emit DECOUPLING. The body's `l` read resolves (exact-lookup) to the registered-but-unemitted
`_cdz_m0` → unbound on recompile → CDZ0101.

**Not a silent miscompile.** `_cdz_m0` is a synth `_cdz_mN` name; source names never collide with it, and
match-arm scopes are separate on recompile, so the fabricated reference is ALWAYS unbound → ALWAYS
CDZ0101-caught → a clean cadenza-target TODO. This is a COMPLETENESS gap, not a correctness hole. (Corrected
from an earlier worry that a name collision could make it silent — it cannot.)

Cadenza-target ONLY: the wasm/rust backends emit a real decision tree; the SURFACE re-emit is what cannot
express "test-a-nested-sub-pattern AND bind-the-whole-slot" (Cadenza has no as-pattern `x@(Lit n)`).

## 3. Approaches

### Approach A — UN-FLATTEN to an inner match (recommended shape)

When a multi-payload variant's arm binds its slots WHOLE and its cont dispatches on a slot, emit the FLAT
outer pattern and push the slot dispatch into the arm BODY as an inner `match`:

```
(match e
  ((Add _cdz_m0 _cdz_m1)                 ; flat: whole slots bound
     (match _cdz_m0                       ; inner: dispatch on slot 0
       ((Lit 0) (simplify _cdz_m1))
       (_       (Add (simplify _cdz_m0) (simplify _cdz_m1)))))
  ((Lit _cdz_m6) (: (Lit _cdz_m6) Exp)))
```

This is the NATURAL nested-match lowering the optimizer flattened; un-flattening restores whole-slot
binders for free and reads value-equivalent (recompile re-flattens to the same decision tree).

- PRO: no reconstruction hackiness; whole-slot binders are genuinely bound; generalizes to arbitrary
  nesting; the emitted shape is exactly what a hand-written nested match would be.
- CON: needs an arm-emitter whose SCRUTINEE is a bound NAME (`_cdz_m0`) rather than a `Core` node. Today
  `emit_match_sum`/`emit_switch_tree` key `env.payloads` by `(Core StructId scrutinee, path)` and emit the
  scrutinee via `emit_expr`. For the inner match the scrutinee is the surface binder name and the cont
  paths are RE-BASED (strip the `[Payload, Elem(0)]` prefix so they are relative to the slot value). This
  is a moderate refactor: parameterize the arm-emitter over "scrutinee surface node + path-prefix under
  which body reads are keyed" so it can run with a name node and a non-empty read-prefix.
- Implementation sketch: at mod.rs:4246, before routing to `emit_switch_tree`, detect that (a) the variant
  is multi-payload AND (b) the arm body reads ≥1 whole slot (a `Core::SumPayload{scrutinee, [Payload,
  Elem(i)]}` read exists — reuse the `node_references`/`collect_*` walk). If so: emit the flat
  `(Ctor _b0 … _bk)` pattern (whole-slot binders already registered), then emit the cont as an inner match
  whose scrutinee-node is the slot binder name `_bi` and whose reads are keyed under the FULL root path
  `[Payload, Elem(i), …]` (so the body's existing SumPayload keys still resolve). Fall through to today's
  flatten when the body reads NO whole slot (no regression).

### Approach B — body-RECONSTRUCT the whole slot (fallback)

Keep `emit_switch_tree`'s flattened arms; for each surface arm whose body reads a whole slot the pattern
destructured/wildcarded, bind it back at the top of the body:

- WILDCARD slot read-whole (`build_arm_pat`'s `choices[path] == None → _`, mod.rs:3358): emit the
  pre-registered whole-slot binder NAME instead of `_` (a binder matches anything = semantically identical
  to `_`, additive/safe). This alone fixes the wildcard arm. (Call this B1 — trivial + independently
  landable if a wildcard-only-read witnessing case is found via `gate --show-declines`.)
- DESTRUCTURED slot read-whole: wrap the body `(let ((_cdz_m0 (<Ctor> <inner-binders>))) <body>)`,
  reconstructing the whole slot from the destructure's inner binders + the matched variant (B2). Needs
  `build_arm_pat` to THREAD OUT a reconstruction plan {whole-slot-path → (ctor, inner-binder-names)} for
  each destructured-but-read-whole slot, which the arm-body emit consumes to emit the wrapping `let`s.

- PRO: localized to `build_arm_pat` + the arm-body emit; no name-scrutinee arm-emitter.
- CON: reconstruction is fiddly (thread the plan; order the lets; handle nested destructures whose inner
  is itself destructured — a recursive reconstruction); less general than A.

### Recommendation

Implement **B1 first** (trivial, safe, independently landable — bind-the-registered-name for a
wildcard-read-whole slot; find its witness with `cargo xtask gate <file> --show-declines --target
cadenza`). Then implement **Approach A** for the destructured case (the cleaner general shape) — the rewrite
case needs A (its `(Add l r)` body reaches the `(Lit _cdz_m3)` destructured arm). Keep **B2** as the
fallback if A's name-scrutinee arm-emitter refactor proves too invasive.

## 4. Correctness argument

- Value-equivalence: A emits the SAME decision (outer variant, then slot dispatch) as the flattened form;
  the inner match on the whole-slot binder re-dispatches identically. The recompile re-lowers the inner
  match into the same Core decision tree the direct path produces. B likewise: reconstructing
  `l = (Lit _cdz_m3)` yields exactly the value the destructure came from (a single-variant wrap of the
  matched payload), so `simplify l` sees the same value.
- Exhaustiveness: A's outer flat pattern covers the variant; the inner match must be exhaustive over the
  slot's sum (the cont's Switch already enumerates its cases + a covering default — carry the same
  covering-arm logic `emit_match_sum` uses for folded variants). B is unchanged from today's exhaustive
  flattened arms.
- No silent miscompile at any stage: today's failure is CDZ0101 (caught). A/B either emit a re-compilable
  program (fix) or, if a shape is still unhandled, DECLINE (reject-don't-miscompile) — never a wrong value.

## 5. Migration / corpus impact

- Target case: `20-structural-editing` "rewrite-then-eval" → todo→pass on cadenza.
- Regression surface: any `--target cadenza` nested-sum match. Verify NO regression on the sum-match corpus
  files: `05-compound-types`, `13-strings`, `17-symbols`, `20-structural-editing`, `26-program-conditions`
  (and a broad cadenza sweep). Use `cargo xtask gate <file> --show-declines --target cadenza` (v-corpus-harness
  #7821) to enumerate before/after decline deltas per file.
- Dual-path value check: compile→cadenza→recompile→run the rewrite case and confirm the value matches the
  direct wasm path (needs a fresh store — `cargo xtask build` first; the sync bumps the runtime hash).
- Level-equivalence: `--opt-sweep` the touched cases (the emit is O-level-independent, but confirm).
- wasm/rust untouched: the change is entirely in the cadenza-backend re-emit
  (`backend/cadenza/mod.rs`); the other backends do not call `emit_match_sum`.

## 6. Fix-point index (for the fresh-context implementer)

All in `implementation/seed/crates/rcdzc/src/backend/cadenza/mod.rs`:
- `emit_match_sum` flat-arm loop: the `SumCont::Switch { path, arms } if path.is_empty()` root + `for arm in
  arms`; whole-slot binder mint+register in the `for slot in 0..arity` block.
- **The fix point: the `SumCont::Switch { .. }` deep-tree branch (~mod.rs:4246)** — routes a multi-payload
  nested cont to `emit_switch_tree`. This is where A gates (un-flatten when the body reads a whole slot).
- `build_arm_pat` (~mod.rs:3239): the `choices[path] == None → b.name("_")` site (~3358) is B1's edit; the
  Tuple/Record/ctor destructure recursion (~3496-3546) is where B2 threads the reconstruction plan.
- `Core::SumPayload` read resolution (~mod.rs:2202): the exact-lookup (2204) is what resolves `l` to the
  registered-but-unemitted `_cdz_m0` today.
- `emit_switch_tree` (~mod.rs:3575) + `emit_nested_switch_chain` (~mod.rs:4276): the tree-walk A must
  either extend (name-scrutinee variant) or bypass for the whole-slot-read case.
