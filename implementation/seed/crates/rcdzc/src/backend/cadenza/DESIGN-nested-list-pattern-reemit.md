# DESIGN — nested list/collection patterns in the cadenza sum-match re-emit

Owner: v-cadenza-backend. Status: SUBSTANTIALLY LANDED — see §10 (2026-09-04). Site A shipped (#7880, §8) and
BOTH original target cases (§1) now round-trip on `--target cadenza`; only a contrived direct-element-payload
residual remains (§10). §1–§6 below describe the ORIGINAL (now-resolved) symptom; read §8/§9/§10 for current
state. (M4a process — see `DESIGN-matchsum-nested-pattern-whole-slot.md`.) Scope: the `--target cadenza` re-emit of a
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

## 7. Investigation notes (2026-09-02) — the list-fold decline is a KEYING MISMATCH, not a missing pattern

Both frontier decline sites are now instrumented with `tracing::debug!` (target `rcdzc::backend::cadenza`):
run `RUST_LOG=rcdzc::backend::cadenza=debug cdz compile … -t cadenza` to see the exact declining read
path / probe. (The `CDZ_LOG` env var scopes it inside the `cargo xtask` pipeline.)

**The list-fold case's exact decline** (from the trace): a `Core::SumPayload` read at
`path=[Elem(0), Payload]`, `cur_ty=Sum{…}` — i.e. reading a LIST element's SUM PAYLOAD (`a` in
`#list((Ast.Int a))`). The nested-read walk descends `Elem(0)` (into the list element, a sum) then hits
`Payload` (the element's variant payload), which it can't project.

**Root cause — NOT simply "emit_match_list lacks nested element patterns":** the element-variant test
`(Ast.Int a)` is a nested `Core::MatchSum` on the element, but the arm body reads `a` keyed from the ROOT
LIST scrutinee (`(list_node, [Elem(0), Payload])`), whereas the nested `MatchSum` registers its payload
binder keyed ELEMENT-relative (`(element_read_node, [Payload])`). The two keys DIFFER, so the read misses
`env.payloads`. The longest-registered-prefix resolution (~mod.rs:2310) DOES find the element binder at the
`[Elem(0)]` prefix (registered by `emit_match_list`), but the remaining suffix `[Payload]` is a sum-payload
step the projection walk cannot cross (site A). So the fix is to reconcile the keying — either register the
element's nested-`MatchSum` payload binder under the ROOT-relative path (`[Elem(0), Payload]`), or teach the
suffix walk to cross a `[Payload]` step by emitting the element's variant sub-pattern in the list pattern
(`(list (Ast.Int a))`) so `a` binds there. The latter is the "nested element sub-pattern" the design's §2
notes `emit_match_list` declines.

**Boundary refinement:** a nested-variant element match over a list that FOLDS to a known-length constant
(`#list((Some v))` on a literal list — probes m3/m4 in `/tmp/lf`) COMPILES today (the fold resolves it
statically). Only a RUNTIME (non-folding) list — here `xs2`, a `fold-list` Call result threaded through
mutual recursion — hits the keying mismatch. And the isolated defs (`fold-list` alone, `fold`-inner alone)
compile — the shape survives only when neither def inlines (full mutual recursion), so a minimal witness is
elusive; work from the full lowered decision tree.

Code change here = the two `tracing::debug!` instrumentation points at the frontier decline sites (§3), so
the exact declining path/probe is observable on demand. The re-emit fix itself remains a later slice.

## 8. Site A LANDED (#7880, 2026-09-02) — keying reconciliation, NOT the guard-hoist

Site A (the list-fold case) shipped as a **keying reconciliation** in `emit_match_sum`, not the fragile
guard-hoist (approach 2). Dumping the ACTUAL Core reaching `emit_match_list` showed the desugar
(`desugar_refutable_ctor_list_elements`) ALREADY emits a round-trippable shape: `(guard (list b0) (match b0
((Ast.Int _) true)(_ false)))` + a BODY re-match `(match b0 ((Ast.Int a) realbody)(_ trap))`. Both guard and
body are plain `MatchSum` on the element binder — the ONLY blocker was that the body re-match's scrutinee
resolves to `SumPayload{root,[Elem(i)]}`, so `emit_match_sum` registered `a` at `(body_scrut,[Payload])` while
the optimizer-composed body read used the ROOT-relative `(root,[Elem(i),Payload])`. **Fix** (mod.rs ~4254,
`emit_match_sum` explicit-variant arm): when the match scrutinee is itself a nested `SumPayload{root,prefix}`,
register each payload binder under BOTH the direct `(scrutinee,path)` AND the composed `(root,prefix++path)`
key. Additive (composed key names exactly this payload), cadenza-only. This is design §7 "approach 1 (align
keys)", which turned out to be the clean+safe fix — approach 2 unneeded. Flips the 20-struct list-fold
todo→pass (value 102).

## 9. Site B (quasiquote `ListLen` probe) — a DEEPER frontier than the ListLen probe alone (2026-09-02)

The remaining 20-struct cadenza todo ("a NONZERO BigInt literal probe in a recursive quasiquote-pattern simp
matches its own constructor"): `(def (simp node) (match node ((quasiquote (* (unquote x) 1)) (simp x)) (other
other)))`. Traced (repro `/tmp/qqb/qq.sexp`), the FULL decision tree reaching `emit_switch_tree` is:

```
LitTest{path:[Payload], probe:ListLen{len:3, at_least:false},
  then_: LitTest{path:[Payload,Elem(0),Payload], probe:Str("*"),
           then_: LitTest{path:[Payload,Elem(2),Payload], probe:Int(1),
                    then_: Leaf(body reads x at [Payload,Elem(1)]), els: Leaf(other)},
           els: Leaf(other)},
  els: Leaf(other)}
```

**Key finding — the ListLen probe is the EASY part; the hard part is that there are NO disc `Switch` nodes
anywhere in the tree.** The element variants (element0 = `Ast.Name`/symbol with a Str payload, element2 =
`Ast.Int` with a BigInt payload) are ASSUMED — encoded ONLY as payload `LitTest` probes at
`[Payload,Elem(i),Payload]`, with the enclosing discriminant constraints FOLDED away (the `Probe` doc: a
ListLen/Str/Int probe is "gated once the enclosing discriminant constraints are satisfied" — here those
constraints were statically discharged by the partial evaluation of `(simp (quote (* y 1)))`, so no `Switch`
survives). So `build_arm_pat` has NO variant info at `[Payload,Elem(0)]`/`[Payload,Elem(2)]` (no `choices`
entry, and `folded_disc` recovery only fires at the ROOT path via `Core::SumNew`). Even after wiring the
ListLen→`(list …)` reconstruction, each element is a MULTI-variant `Ast` with only a payload lit-probe → it
hits `build_arm_pat_inner`'s `_ => decline` ("cannot destructure this value to reach a deep-match
constraint"). So a pure ListLen slice would NOT flip this case and has no other witness (→ not a meaningful MR).

**What Site B actually needs (the deep piece):** VARIANT RECOVERY FROM THE PROBE PAYLOAD TYPE — at a
multi-variant sum path with no `choices` entry but a deeper payload lit_choices key, recover the variant whose
payload TYPE matches the probe kind (a `Str` probe → the variant with a Str payload; `Int`/BigInt → the BigInt
payload variant), and cross into it (emit `(Ast.Name "*")` / `(Ast.Int 1)`). SOUNDNESS is the risk: it is
correct only if the variant is UNAMBIGUOUS (exactly one variant of the sum has a payload of that kind); an
ambiguous sum (two Str-payload variants) must DECLINE. Because the cadenza gate is VALUE-only (not
byte-idempotent), emitting `(Ast.Name "*")` (whose re-lowering ADDS a redundant disc check the original tree
folded) is fine IF value-equivalent — and a wrong variant guess shows as a VALUE MISMATCH in the local A/B
(not a silent miscompile that escapes the gate). So the safe impl path: (1) ListLen→list-pattern in
`emit_switch_tree` + `build_arm_pat` (thread a `list_choices`/`(len,at_least)` map, emit `(list e0 …
e{len-1} [.. rest])`, recurse elements at `[path,Elem(i)]`); (2) unambiguous variant-recovery-from-probe-type
in `build_arm_pat_inner`; (3) gate strictly on unambiguity, then A/B the whole 20-struct corpus for value +
zero regression before landing. This is a dedicated fresh-context effort (3 interlocking pieces + a soundness
gate), NOT a bounded tick — deferred. Repro `/tmp/qqb/qq.sexp` (`cdz compile qq.sexp -t cadenza`; trace with
`RUST_LOG=rcdzc::backend::cadenza=debug`).

## 10. STATUS 2026-09-04 — both original target cases PASS; only a contrived residual remains

Re-verified the two §1 target cases on `--target cadenza` (hop1 → hop2 recompile → run):

- **list-fold** ("a mutually-recursive fold matching a rebuilt list with a payload binder…", 20-struct:762)
  — hop1 emits (966 B), hop2 recompiles, runs **102** at n=2. Site A's keying reconciliation (#7880, §8) fixed it.
- **quasiquote-simp** ("a NONZERO BigInt literal probe in a recursive quasiquote-pattern simp…", 20-struct:533)
  — hop1 emits (934 B), hop2 recompiles, runs **40**. The Site B frontier §9 deferred no longer blocks this
  case (the `(simp (quote (* y 1)))` input partial-evaluates so the surviving tree is re-emittable). Passing.

So the slice as originally scoped (its two motivating corpus cases) is DONE — do NOT re-implement §1–§6/§9's
"deferred" plan expecting those cases to decline.

**Remaining residual (LOW priority, no real corpus need):** a DIRECT read of a list element's SUM PAYLOAD in
a match arm body still declines at `mod.rs:2687` (the `Core::SumPayload` read walk, `path=[Elem(0), Payload]`,
`step=Payload`). Contrived repro: `(match #list((pick n)) (#list((C.R x)) x) (_ -1))` with `pick` a
recursion-forced producer of a 2-variant sum. This is the same §7/§8 keying area, but Site A's dual-key
registration (in `emit_match_sum`) covers the fold's desugared guard+body shape, not this direct
single-element-list read (whose payload binder is not registered under the composed `[Elem(0), Payload]` key).
No CURRENT corpus case hits it (the target cases pass), so it is a contrived edge — pursue only if a real case
surfaces; the fix would extend the §8 dual-key registration to the direct-element-payload read path.
