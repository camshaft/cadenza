# Perceus reclaim placement for heap-payload sum-match and self-loop tails

Status: DESIGN (v-core-opt, 2026-08-27). The emit/placement half of the co-design seeded by v-runtime's
`DESIGN-loop-and-sum-heap-reclaim-alias-analysis.md` (#3833). That doc frames the two biggest leak
classes — self-loop-tail (~333) and heap-payload sum-match (~195) — as one liveness-at-drop-point
problem: a shell/old-value deep-drop is suppressed today because the analysis cannot prove no reference
into the shell is live past the drop. This doc specifies the decision the emit uses to prove it, and
where the drop is placed, so the reclaim widens without admitting a use-after-free.

The correctness bias is unchanged from #3833: a leak beats a UAF. A wrong "safe to reclaim" is a
double-free (trap); a wrong "unsafe" is a leak (value-correct). The decision is a conservative
under-approximation that only reclaims when both clauses below prove safety.

## The two existing analyses and what each covers

Both live in `backend/wasm/select.rs`.

- `binding_escapes(body, binder)` (@1280, worker `_dup_aware` @1308) — the real escape analysis over a
  `LocalRef`/`Param` binder. It threads borrow-vs-consume (a `Proj`/`ListLen`/… operand is a borrow; a
  result, call arg, or constructor element is a consume), and in its `dup_aware` form it treats a
  consume that is a Perceus retain site (`dup_sites`) as non-escaping (the dup gave the consumer its own
  reference, so the binding's own reference survives and the owner reclaims it).
- `arm_borrows_heap_subvalue(arm_body)` (@15861) — a syntactic walk for a heap-typed
  `Proj`/`SumExpect`/`SumPayload` read that appears in a consume/result position. Position-aware:
  a match scrutinee and a projection operand relax to borrowed.

Neither alone closes the gap (v-runtime's three trap witnesses `mts1`/`mmx1`/`rrb1` prove it), for two
representation reasons established with v-runtime:

1. **Binder representation is mixed.** A single-use scalar payload read lowers to an *inline*
   `Core::SumPayload` node (no binder id) — only `arm_borrows_heap_subvalue` sees it. A multi-use or
   `let`-bound heap payload materializes to a *slot* → `LocalRef` — `binding_escapes` sees it. Example
   (`mts1`): `pair` is source-level `let`-bound and used twice (matched, then passed to `Map.insert`),
   so it is a materialized `LocalRef`, and `binding_escapes` correctly reports it escapes into
   `Map.insert`. The tuple-element binders `c`/`s`/`c2`/`s2` are inline scalar reads.
2. **FBIP reuse is invisible to escape analysis.** `collect_dup_sites` (@2075) computes retain sites
   from source structure alone. A *single-use* consuming op takes the FBIP fast path — it reuses its
   operand's storage in place (a uniquely-owned `vec-push`/`map-insert`/tuple-build mutates the cell). A
   rebuild `(tuple (. t 0) (. t 1))` where `t` is single-use reaches `t` only through borrowing `Proj`s,
   so `binding_escapes`/`arm_borrows_heap_subvalue` both say "safe" — but the constructor's FBIP reuse
   silently takes `t`'s cell into the result. A subsequent shell deep-drop then frees a cell now aliased
   into the live result: the `mts1` double-free.

## The reclaim decision (heap-payload sum-match, site B)

For a `MatchSum`/`MatchList`/`SumExpect` whose scrutinee is a proven-owned temporary
(`heap_operand_ownership == Owned`) with a heap payload, deep-drop the shell after the arm iff BOTH:

- **escape-clean** — every heap payload sub-value the arm destructures is non-escaping:
  - a materialized `LocalRef`/`Param` payload binder has `binding_escapes_dup_aware(arm_body, b, false,
    Some(dup_sites)) == false`; and
  - the inline-read residue is `arm_borrows_heap_subvalue`-clean.
- **reuse-clean** — no FBIP reuse in the arm takes a cell reachable from the shell being dropped:
  no constructor (`Tuple`/`SumNew`/`ListNew`/`MapNew`/…) in the arm reuses, via the single-use FBIP
  fast path, a payload cell reachable from the shell. This is the new clause; see below.

When both hold, reclaim using the proven vehicle already in the `SumExpect` gate (@13031): for each
consumed heap payload child at a dup site, `dup` it (rc++) before it flows into the consuming op, then
deep-drop the shell — the drop cascade decrements the dup'd child back to a live rc, and reclaims the
shell storage plus every non-escaping child. A child that is a pure borrow (destructured to scalars)
holds no handle and is reclaimed by the cascade with no dup. This is exactly the discipline the
`SumExpect` path applies for `Option.expect`; the increment ports it to the general `MatchSum`/
`MatchList` arms, replacing the crude `sum_has_only_scalar_payloads` all-scalar floor
(@7791 tail, @13475 non-tail).

### Exposing reuse-clean

`collect_dup_sites`/`mark_binder_dups` already decide, per consuming occurrence, dup-vs-FBIP-fast-path.
The reuse-clean clause reuses that decision: a payload cell is FBIP-reused iff a same-shape constructor
in the arm consumes it through the single-use fast path (not a dup site). The predicate walks the WHOLE
arm body (a rebuild nested in a `let`/`if`, not only the arm's tail) for a constructor whose elements are
`Proj`s of a payload sub-value where that sub-value is not a dup site — i.e. the storage the constructor
will reuse is the shell's own. If found, the shell is not deep-dropped (the reuse already reclaimed/
aliased the cell; a second drop double-frees). This is a source-structure predicate, matching the
determinism requirement in `spec/capabilities/memory-and-resource-model.md` (reuse-is-not-observable).

The residual scope is exactly CONSTRUCTORS reusing a `Proj`-reached cell. A consuming-op FBIP
(`List.push`/`Map.insert`/`Set.insert`/`vec-update` on a uniquely-owned payload) is already covered by
escape-clean — the op consumes the binder, so `binding_escapes`/`arm_borrows_heap_subvalue` flags it —
so reuse-clean does not need to re-cover it.

*Resolved with v-runtime (review of 13bae486a):* by the escape analysis, all three current trap
witnesses are already ESCAPE-caught — `mts1`'s `pair` escapes into `Map.insert` (materialized `LocalRef`,
`binding_escapes`), and `mmx1`/`rrb1` thread their compound state into `resume` (escape). So escape-clean
ALONE declines all three. reuse-clean is therefore DEFENSIVE-FOR-COMPLETENESS — it guards a single-use
`Proj`-rebuild shape not present in the current trap set, and is strictly safe (it only declines more).
Increment 2 confirms this empirically: run `mts1`/`mmx1`/`rrb1` with escape-clean ONLY and verify each
still DECLINES (stays `known-leak`) on the debug runtime `052KQzQP`. If any trap RECLAIMS under
escape-only, that pins the case that genuinely needs reuse-clean and validates the predicate; if all
decline, reuse-clean guards a not-yet-witnessed shape. Either way it stays in.

## The reclaim decision (self-loop tail, site A)

`list_shell_reclaim_slot` (@15809) returns `None` for `TailPos::Tail(Some(_))`: the self-loop back-edge
is a `br` to the loop top that never reaches the post-match drop, and the scrutinee stash slot is reused
next iteration, so the walked node is never reclaimed. A single drop before the back-edge does not reach
rc 0 because the loop body dups the walked param for its multiple uses (match scrutinee, head read, rest
read), so its rc at the back-edge is ≥ 2 (v-runtime verified across three attempts).

The walked param is not wholly dead at the back-edge: its tail escapes into the recursive call (the next
iteration's param). But the cons SPINE cell is separable from its escaping tail child — the classic FBIP
list-walk: the spine cell is dead once the head is read and the tail is handed on. So site A reclaims the
SPINE cell on the `br` path, balanced against the multi-use dups: emit a drop of the spine after the
per-iteration dups net out, so each iteration nets zero. The tail child is retained (it becomes the next
param); the head is a scalar copy. The placement is the back-edge analogue of the post-match shell drop,
targeting the spine slot rather than the whole scrutinee.

**Empirically confirmed (2026-08-28, WAT dump of the fold repro `(go (: xs (List Int64)) (: acc Int64)) =
(match xs ((list) acc) ((list h .. t) (go t (+ acc h))))`):** the emitted `go` loop dups the walked param
`xs` (`local 0`) TWO-TO-THREE times per iteration via `call $dup` — once for the `vec-len` dispatch, once
for the `vec-get 0` head read, once for the `vec-drop 1` tail (`t`) — then does `local.set 0` (xs ← tail);
`br 1`. There is NO `drop` of the old spine cell before the store, so each iteration leaks its cons cell
(length-4 walk ≈ 9 objects). `list_shell_reclaim_slot` (now @16146, drifted from @15809) returns `None`
for `TailPos::Tail(Some(_))` (the loop back-edge) — the gate that declines the reclaim. The back-edge
emit is `emit_loop_iteration` (@7849): it evaluates the new args, then `local.set` each param slot
(`param_slots`, incl. `xs ← t`), then `Br(loop_top)`.

**RESOLVED with v-runtime (rc-read, 2026-08-28) — it is OVER-DUP, not a missing drop; fix = CONSUME-LAST
ORDERING + last-use-no-dup, two INSEPARABLE parts.** Per-op ownership (runtime.wit + select.rs): `vec-len`
BORROWS the walked param, `vec-get` BORROWS it (head `dup`'d out separately), `vec-drop(xs,1) → t`
CONSUMES it (op 72, reuses the spine into `t` at rc==1 via FBIP). So the walked param needs ZERO dups: rc 1
in, borrow-read by len/get, then CONSUMED by `vec-drop` (rc 1→0, reuse). The leak is the emit OVER-DUPPING:
`mark_binder_dups_inner`'s Param arm marks the walked param at `consuming=true && live_after=true` — the
CONSUME (`vec-drop`) is forced to `dup` because a SIBLING BORROW (`vec-get` head) is treated as
simultaneously live (the sequential-group "all siblings live" model). WAT confirms `vec-drop`(@156)
currently emits BEFORE `vec-get`(@161), so at the consume the head is not yet read → live_after=true → dup;
that dup orphans at the `xs ← t` reassign = the leak (and inflates rc so `vec-drop` path-copies instead of
reusing). This is why v-runtime's 3 prior back-edge-DROP attempts did nothing (rc ≥ 2 from over-dups; no
separate spine cell — `vec-drop` already consumes/reuses it).

THE FIX (two parts, ATOMIC — must land together):
- **PART 1 — CONSUME-LAST emit ordering (v-core-opt emit lane).** `emit_loop_iteration` (@7849) evaluates
  the recursive-call args left-to-right (push @~7890) then pops reverse into param slots (@~7903). For
  `(go t (+ acc h))`, arg0 `t` (`vec-drop`, CONSUME) evaluates before arg1 `(+ acc h)` (`vec-get`, BORROW).
  Reorder so every arg that BORROWS the walked param is evaluated BEFORE any arg that CONSUMES it (a
  coordinated eval+pop reorder — the pop order must track the new stack order; semantics-preserving since
  the arg reads are independent, v-runtime confirmed `h` before `t` changes no value). After the reorder
  `vec-drop` is the GENUINE LAST use of the walked param.
- **PART 2 — last-use-no-dup (`mark_binder_dups` refinement).** A CONSUME of the binder that is the LAST
  use — only live-after siblings are BORROWS that already completed — needs NO dup. Refine the
  sequential-group model to sequential liveness (a borrow RELEASES at its read, so a borrow sequenced
  before a consume is not live across it). Post-PART-1, `vec-drop`'s live_after has no walked-param borrow
  → false → no dup.
- 🚨 **PART 1 IS THE UAF-SAFETY GATE.** Suppressing the consume's dup WITHOUT the reorder is a UAF: if
  `vec-drop` (frees/reuses xs) stays before `vec-get` (borrow) and the dup is gone, `vec-get` reads a freed
  xs. Gate PART 2 on "the consume is the LAST use of the binder (all borrows sequenced before it)" — which
  PART 1 guarantees. Do NOT land PART 2 without PART 1.

NET: xs rc 1 → borrows (rc 1) → `vec-drop` consumes the sole ref (rc 1→0, FBIP-reuse into `t`) → ZERO dups,
ZERO leak, reuse-not-path-copy. Generalizes across fold/count/walk (walked param = N borrows + 1
tail-consume; make the consume last, no dup). Biggest remaining leak class (~333). Acceptance (v-runtime
co-verify): fold/count/walk → 0 value-correct + no-trap + reused-tail-not-double-freed, AND a counter-case
where a borrow WOULD-be-after-consume stays correctly dup'd (proves the last-use gate). Implement PART 1 +
PART 2 atomically; trace the reordered WAT (`vec-drop` last, no dup, reuse); circulate the diff.

**OUTCOME (implemented, PR #4139, 2026-08-28 — the SOUND guard + two verification tools the object-census
lacks).** The two parts landed as: (i) the CONSUME-LAST reorder in `emit_loop_iteration` — detect the
consuming arg STRUCTURALLY (its value is a `Core::SumPayload` whose path ends in `RestFrom` = the runtime
`vec-drop` consume; NOT `binding_escapes`, which calls the fresh tail a borrow — that miss no-op'd two
attempts) and evaluate it LAST (coordinated eval+pop). (ii) an Emit SKIP-SET (by wasm slot) marks a
loop-carried param whose preservation dups (`emit_binder_ref` retain + the `RestFrom`-step dup at the
`SumPayload` emit) are skipped — gated NOT at `mark_binder_dups` but at those two emit sites. THE SOUND
GATE is `count_param_consumes`: skip iff the param's SOLE consuming use is the reordered-last `RestFrom`,
counting consumes across ALL args + ALL nesting = the CONSUME-BUT-PRODUCE-FRESH op CLASS (`List.concat`/
push/update, `Bytes.concat`, `Map.insert`/remove, `Set`-ops) ∪ `RestFrom` ∪ escapes (`Call`/`CallClosure`/
ctor). `binding_escapes` ALONE is UNSOUND — it treats the whole consume-but-fresh class as a borrow, so a
NESTED consume (the `INVERSION` case: `count-after` Call, or a sibling `RestFrom`/`Map.insert` of the
param) is missed → the dup wrongly skipped → an rc imbalance. A param with count > 1 KEEPS its dup.
🔑 TWO VERIFICATION TOOLS the static live-objects census cannot provide (both now standard for reclaim
work): (a) VALUE-WRONG GREP — a miscompile reports `expected (: X), ran/got Y` with NO `trapped:`, so it
hides among leak-count mismatches; grep for any FAIL that is NOT a `live-objects mismatch`. (b) FLAP-
DETECTION — run the whole corpus TWICE; a live-objects count that DIFFERS run-to-run is a census-hidden
rc-UNSOUNDNESS (an order/allocation-dependent imbalance), even when the value is correct. The first
(unsound) guard made `INVERSION` flap 1↔2; the sound guard keeps its nested-consume dup → `INVERSION`
stable at value-3, leak 36→16. RESULT: 143 value-correct leak reductions, 0 value-wrong, no flaps.

## What actually leaks: generic-sum instantiation × heap payload

Breaker localized the leak family precisely (#3865, `df` quad in `05-compound-types.sexp`). A
construct-then-destructure over a MONOMORPHIC sum whose shape mirrors `Option` at a heap payload
(`df1`) DEFORESTS — the construct fuses with the immediate destructure, no shell is allocated, nothing
to reclaim. The SAME shape as a GENERIC sum (`(type (GBox a) …)`) at a heap payload (`df2`) does NOT
fuse — it allocates the shell and leaks 3 (identical to `Option`'s `d4`). Generic-at-scalar (`df3`) and
`Option`-at-scalar (`df4`) both deforest. So the leak family is exactly **generic-sum instantiation × a
heap payload**; prelude-ness is incidental (`Option`/`Result` leak because they are generic). This
scopes the reclaim increment: the shells that reach the reclaim gate are the instantiated-generic ones
the fusion pass leaves un-deforested.

## A second site: the extraction-op retain (site B′)

v-runtime + breaker localized a distinct leak mechanism that shares the sum-shell shape but is NOT the
construct-then-destructure gap above (#3874/#3875/#3876/#3878). A fallible read — `List.at`, `Bytes.at`,
`Map.lookup`, `Bytes.slice` — returns a runtime `Option` whose discriminant depends on bounds, so it
CANNOT deforest: it builds a real `Some` shell AND `dup`s the extracted compound into that shell
(`heap_operand_ownership` @16069-71). When the arm unwraps the `Some` and the shell is deep-dropped, the
drop takes the shell's own ref (rc 2 → 1) but the extracted payload survives at rc 1 — a leak of exactly
the payload's cells. The witnesses:

- `lar1` (`List.at`-extracted inner list, borrow-only via `List.len`) leaks 3; `lar2` (scalar element)
  is the 0 control.
- `mlr1` (`Map.lookup`-extracted, borrowed) leaks 3 identically — so it is the Option-shell extraction
  family, not `List.at`-specific.
- `mlr2` (extracted vec CONSUMED via `List.concat`) STILL leaks 3 — the extraction's retain is never
  paired with a drop REGARDLESS of whether the payload is then borrowed or consumed.
- `mlr3` (a plain tuple projection of a list field, no `Option` shell) is 0 — confirming the leak is
  specifically the `Option`-shell-mediated extraction.
- `xar1`/`xar3` (a handler ARM doing its own extraction) leak PER DISPATCH (`xar1` = 6 at 2 dispatches =
  2× `lar1`), so the fix must reach ARM scope, not only straight-line def bodies.
- `osx3` (`Bytes.slice` wrapping a FRESH view in a `Some`) leaks 2 — an adjacent cell that may flip for
  free if the release is at the `Some` unwrap.

The fix is orthogonal to escape/reuse-clean: it must RELEASE the extraction-op's dup-retain when the
`Some` is unwrapped, on BOTH the borrow path (the shell deep-drop) and the consume path (`mlr2`). Open
with v-runtime (their runtime-reclaim call): place the release at the `MatchSum`/`SumExpect` `Some`
unwrap (drop the extra retained ref once the payload is bound), or at the extraction op itself (do not
`dup` into the shell when the `Some` is a dead temporary consumed by the immediate match).

## Acceptance and fence (corpus-pinned by breaker #3863/#3865/#3874/#3875/#3876/#3878, on the debug runtime)

- Reclaim-to-zero (flip `(live-objects known-leak N)` → `(live-objects 0)` in the SAME PR, per v-nix):
  `d4=3` (minimal Option), `dm1=7`/`d3=5` (scaling), `drs1=3` (Result), `drs2=4` (nested Option),
  `ap1=3` (arm/handler position, in `14c-effects-and-handlers.sexp` beside the fence), `df2=3` (the
  generic-user-sum witness); plus the self-loop `fold`/`count` family for site A. (Note the corpus
  renames that dodge handler-state prefix collisions: the acceptance `rs1`/`rs2`/`rs3` are `drs1`/`drs2`/
  `drs3`, and `d2` is `dt2` — the un-prefixed `rs*` in the corpus are UNRELATED handler-state cases.)
  Extraction-family (site B′): `lar1=3`, `mlr1=3`, `mlr2=3`, `xar1=6`, `xar3=3`, `osx3=2` → 0.
- Fence (must STAY at its current value — leaking is acceptable, a UAF is not, and a control must not be
  over-corrected): the three trap witnesses `mts1`/`mmx1`/`rrb1` stay `known-leak` (must NOT be reclaimed
  until the reuse-clean predicate proves them safe); the zero-controls `dt2`/`d6`/`dst2`/`dst5`/`dst6`/
  `lar2`/`mlr3` (and the deforesting `d5`/`drs3`/`df1`/`df3`/`df4`) stay `(live-objects 0)`. Note `d6` is a
  DEFORESTATION control (a directly-constructed `Option` compound emits ZERO sum ops → no shell to
  reclaim), NOT a witness that the reclaim works — the reclaim witnesses are the extraction-built shells
  (`lar1`/`mlr1`/…) which cannot deforest.
- Repros: the acceptance/`df` rows in `05-compound-types.sexp`; `ap1`/`mts1`/`mmx1`/`rrb1` in
  `14c-effects-and-handlers.sexp`; site-A minimal
  `(def (go (: xs (List Int64)) (: acc Int64)) (match xs ((list) acc) ((list h .. t) (go t (+ acc h)))))`
  (leaks 9 at length 4). Verify on the debug runtime `052KQzQP` with `--report-live-objects`.

## Alternative / complementary: deforest the instantiated-generic path (breaker #3865)

The reclaim-placement increments below reclaim a shell that WAS allocated. A complementary attack is to
stop allocating it: make the generic construct-then-destructure fuse the way the monomorphic one does
(`df2` → read like `df1`). That eliminates the leak at its source for the pure construct-then-immediately-
destructure subset (`d4`/`dm1`/`d3`/`df2`), with no reclaim needed. It does NOT cover shells whose
payload genuinely escapes or is threaded (`ap1`, and the `mts1`/`mmx1`/`rrb1` shapes) — those still need
the reclaim decision. So deforestation and reclaim-placement are complementary: fusion removes the
fusable subset, reclaim covers the rest. Deforestation likely lives in the monomorphization/inlining
pass rather than the shell-reclaim gate; it is out of this doc's emit-placement scope but recorded here
as the cheaper fix for the fusable cases if that pass's owner takes it.

## Increment order

1. DONE (#3877, merged, v-runtime-verified 0-delta): the two `MatchSum` gates (@7791 tail, @13475
   non-tail) share one `sum_shell_reclaim_ok` helper (the common 5-clause predicate). No behavior change.
   The tail gate keeps its `!arms_tail_call` at the CALL SITE — it needs the `MatchSum` decision tree and
   is NOT equivalent to `TailPos::Tail(Some(_))` (a `Tail(Some)` match whose arm is a constructor still
   has a valid reclaim point), so threading `TailPos` generically would NOT be no-delta. The `SumExpect`
   gate (@13031) stays as-is — it is the dup-then-deep-drop TEMPLATE, structurally different.
2a. Widen `sum_shell_reclaim_ok` past the all-scalar floor to admit a COMPOUND heap payload when the arm
   is escape-clean (every heap payload sub-value non-escaping: `binding_escapes_dup_aware` over a
   materialized `LocalRef` binder + `arm_borrows_heap_subvalue` over inline reads) AND reuse-clean (no
   arm constructor FBIP-reuses a shell-reachable cell). Re-enable the (currently no-op)
   `collect_shell_reclaim_child_dups` so consumed compound children are `dup`'d before the deep-drop.
   Flips `d4`/`dm1`/`d3`/`drs1`/`drs2`/`ap1`/`df2` → 0; `mts1`/`mmx1`/`rrb1` stay `known-leak`. Marker
   flip in the same PR.
2b. Release the EXTRACTION-OP retain (site B′) on `Some` unwrap. Split into two stages by escape-clean:
   - **Stage A (DONE, #3989 merged, v-runtime co-verified):** reclaim the escape-clean extraction-`Some`
     shells — the borrow-to-scalar and `None`-path family. Flipped `lar1`/`mlr1`/`xar1`/`xar3`/`osx3`/`xop2`/
     `xop3`/`xop1` → 0 (60 corpus cases), by widening the compound-reclaim branch to admit a fallible-
     extraction scrutinee whose payload is at rc ≥ 2 (the extraction dup-retains it, so any FBIP rebuild
     path-copies — no alias). `mts1` reduced 6 → 3, `mmx1`/`rrb1` stay `known-leak` (fence held).
   - **Stage B (pending v-runtime placement call):** the CONSUME path — reclaim an escape-clean=FALSE
     extraction-`Some` whose payload is moved into a consuming op. Post-Stage-A this collapses to `mlr2`
     alone (`known-leak 3`): `(Some inner) → (List.len (List.concat inner (list 7)))`, `inner` consumed by
     `List.concat`. 🚨 SAFETY: `resume` is REDUCED AWAY before Core (`reduce_handle`/`splice_context` in
     `effects.rs` splice the resume value into the continuation context at the perform hole — there is NO
     effect/resume node in Core), and `collect_consuming_payload_sites` marks a call-arg/tail/return
     position (where a spliced continuation routes, via `Core::Call`/`CallClosure`) as consuming
     IDENTICALLY to `List.concat`. So the dup-marking cannot distinguish a resume-thread from a pure
     consume, and select.rs cannot key on `resume`. The current fence (`mmx1`/`rrb1`) holds ONLY because
     they are NON-extraction (threaded-state `Some`), excluded by the extraction-scrutinee gate — NOT by
     any resume check. Proposed fence for Stage B: reclaim iff every escaping payload binder is consumed
     by a PURE structure-building op from a CLOSED allowlist (`ListConcat`/`ListPush`/`MapInsert`/
     `SetInsert`/`vec-update`/`BytesConcat`/`SetAlgebra`), NOT via the default consuming arm
     (`Call`/`CallClosure`/`HostCall`). This structurally excludes both a future extraction-payload-into-
     resume case and `mmx1`/`rrb1`, while reclaiming `mlr2`. Open with v-runtime: (a) whether tail-
     resumptive extraction-into-resume is even a UAF (single one-shot splice → dup-on-escape may already
     pair 1:1), and (b) whether the pure-consumer allowlist is the right fence given `resume` is
     Core-invisible. Unblocks v-rb's nested-lift (`el8`/`eln1-3`).
2c. The remaining `MatchSum` shell leaks after Stage A+B, three DISTINCT mechanisms (proposed order —
   tractable/safe first, UAF-critical last; each circulated + co-verified before its emit):
   - **2c.1 REPEAT-UNWRAP (proposed FIRST — no `resume`, no threaded state, pure straight-line).**
     `xop4`/`xop5`/`ruw1`/`ruw2`/`ruw3` all leak 3: a FRESH compound-payload sum (`Option.Some`/`Ok`/
     generic `GFull`) bound to a `let` and MATCHED N TIMES. The scrutinee is a SHARED local (rc = N uses),
     each unwrap `dup`s the payload to hand the arm a borrow, but only ONE shell-reclaim balances — the
     leak is the `(N-1)` un-released per-unwrap `dup`s (the ref model's `(#unwraps-1)` term; `ruw3` matched
     THREE times still reads 3 because the census counts OBJECTS not refs, so a half-balance HIDES). The
     scrutinee is NOT an extraction (no dup-retain) and each arm BORROWS to a scalar (`List.len`). Fix
     (v-runtime's runtime-reclaim call, my emit placement): model the `Some`-unwrap as releasing the
     per-unwrap `dup` at the unwrap site so N unwraps net one shell + one payload, not N. The current gate
     declines because a multiply-used `let` scrutinee is not `heap_operand_ownership == Owned` at any single
     match; the reclaim must be at the LAST unwrap / the `let`-drop, accounting for all N `dup`s.
     🚨 ACCEPTANCE needs a REF-LEVEL assertion or a distinct-structure witness (breaker), not the object
     census — a half-balance (release N-2 of N-1) reads the SAME live-objects count as full balance.
   - **2c.2 mts1 RESIDUAL (extraction `Some p`, payload PROJECTED to scalars + arm CONSTRUCTS a fresh
     tuple).** `mts1` = 3 after Stage A. The `Map.lookup` `Some p` (a tuple) is matched
     `((tuple c s) (tuple (+ c 1) (+ s v)))` — `p` is projected to scalars `c`,`s` (BORROW) and the arm
     REBUILDS a fresh tuple from those scalars. Stage A/B decline it because the rebuild trips reuse-clean
     (`sum_cont_arm_constructs_compound`), yet the rebuild reads only SCALARS — it cannot FBIP-reuse `p`'s
     cell, and `p` is at rc >= 2 (extraction dup) so any reuse would path-copy anyway. So reuse-clean is
     OVER-conservative here. Candidate fix: refine reuse-clean to not flag a construct whose operands are
     all scalar-projections of the (rc >= 2) extraction payload. 🚨 `mts1` is the tick-27a TRAP witness —
     re-verify empirically that the residual reclaims value-correct + no-trap BEFORE trusting the refinement;
     the fresh `pair` threaded into `resume` + `Map.insert` is a SEPARATE value (not `p`), so reclaiming
     `p`'s dead shell should not touch it, but confirm on the debug runtime.
     - **RE-VERIFY 2026-08-27 (reactivation tick).** Current main: `mts1` is now `known-leak 2` (a later
       increment — 2c.1 #4051's borrow-classify arm — reduced it 3→2). Re-ran `--case mts1` ×3: **PASS 3/3,
       NO trap, NO flap** — the `2` is deterministic (not a hidden rc-unsoundness), so the fence is holding
       cleanly and there is no regression to chase; the residual is purely a MISSED reclaim.
     - **SHARPENED MODEL (supersedes "reuse-clean over-conservative").** The tick-27a trap came from trying
       to REUSE/FREE `p`'s cell. But if `Map.lookup` returns an OWNED payload duped to a DISTINCT cell (m
       retains its own ref → `p` at rc≥2), then the correct reclaim is NOT reuse — it is a plain missing
       **DROP** of the extraction-duped `p` at arm end (decrement to release the dup; m's ref survives). That
       is the SAME missing-let-drop shape as 2c.1 (#4051), specialized to an extraction payload projected to
       scalars, and it is SOUND (a decrement, never a free of a shared cell). Freeing the shell (what tick-27a
       did) is the UNSOUND variant — it double-frees when lookup borrows-or-shares into m.
     - **BLOCKING ALIAS QUESTION → v-runtime (asked this tick).** Soundness hinges entirely on: does
       `Map.lookup(m,k)` return the value as an OWNED/rc-incremented distinct cell (→ missing-drop is sound),
       or a BORROW into m's CHAMP node storage (→ ANY reclaim of `p` while m is live+threaded = UAF)? The
       CHAMP map lives in the runtime module (v-runtime's lane), so this is theirs to answer. NO emit change
       until answered. If OWNED: land a scalar-projection-only missing-drop for the extraction payload,
       gated exactly like 2c.1, acceptance = value-wrong-grep + flap-detection on `052KQzQP`.
   - **2c.3 THREADED-STATE (`mmx1`/`rrb1`, NON-extraction, UAF-CRITICAL — proposed LAST).** The handler
     STATE (`mmx1`: `Option (Tuple)`, `rrb1`: a `Tuple`) is matched, projected, a fresh state built, and
     the OLD state shell is dead once the NEW state is produced and threaded into `resume` as the next
     state. Reclaiming the old state shell requires proving it dead at the arm — but `resume` is REDUCED
     AWAY before Core (`reduce_handle` splices it), so the "old state is replaced" signal is invisible at
     select.rs, exactly like the Stage-B resume-thread. The scrutinee is NON-extraction (a threaded param),
     so the extraction gate correctly excludes it today (that is why it is fenced). A reclaim here needs a
     dup-on-escape on the threaded state with a structural fence that resume-invisibility does not defeat —
     OPEN with v-runtime, likely the hardest of the arc; defer until 2c.1/2c.2 land.
   - **2c.4 ZIP-SHAPE / DUAL-SPINE TRUNCATION EXIT-DROP (`05-compound` ZIP=3, UNZIP=10, matmul=20 ≈ 33).**
     A dual-spine walk (`zip-sum`) matches BOTH lists in nested matches and recurses on both tails, exiting
     at whichever spine empties first. Empirical characterization on `052KQzQP` (2026-08-27c, scratch probes,
     deterministic — no flap):
     - Leak is keyed to WHICH exit arm fires, and is a FIXED count per exit — **INDEPENDENT of list length,
       unwalked-tail length, productive-step count, AND whether the lists are compile-time-constant
       (#4270-immortal) or runtime-built** (all four confounds varied; count unchanged):
       - inner `((list) acc)` — ys empties while xs was just decomposed to `xh`/`xt`: **leak 3**
       - outer `((list) acc)` reached after walking to equal length: **leak 2**
       - outer `((list) acc)` reached immediately (xs empty on entry): **leak 1**
     - 🚨 **This DISPROVES the earlier "unwalked tail" hypothesis** — the unwalked tail IS reclaimed (leak
       does not grow with it). The residual is a FIXED set of dead bindings live at the RETURN arm that the
       arm fails to drop: the `+2` of the inner-arm case over the immediate case = exactly the `xh`/`xt`
       decomposition of the enclosing xs-match that the ys-empty arm returns WITHOUT consuming (`acc` alone
       escapes). So the lever is: **at a match arm that returns a value NOT derived from a heap binder bound
       by an ENCLOSING match still in scope, DROP that binder** — gated on the binder not escaping the arm.
       This is the EXIT-DROP arc, now the near-term priority (concierge 2026-08-27: reclaim co-design before
       deep-mark). It does NOT depend on deep-immortal-marking.
     - **OPEN for v-runtime (circulated this tick, BEFORE any emit):** confirm the object identity of the 3 /
       2 / 1 (are these boxed elements + list handles, or per-frame artifacts?), and confirm dropping the
       enclosing-match binder at the sibling-return arm is sound (the binder is dead there; the returned
       `acc` is a disjoint scalar). Acceptance = value-wrong-grep + flap-detection; fence = must not touch the
       recursive arm (where `xt`/`yt` ARE consumed).
     - **WAT EVIDENCE (2026-08-27, `cdz compile /tmp/zipprog.sexp -t wasm` → `wasm-tools print`).** `zip-sum`
       is func 16 `(param i32 i32 i64)` = (xs-vec, ys-vec, acc). The list is a **vec-of-arr with boxed-int
       elements** (`box-int`/`get-int`/`vec-get`/`vec-drop` imports; a `(List Int64)` element is a heap box).
       The **recursive arm emits `call vec-drop` for BOTH `xs` (local 0) and `ys` (local 1)** before the
       `return_call` — proving both operands are owned-and-droppable in this frame. **The two empty-match
       exit arms (`((list) acc)`, each `local.get 2; return`) emit NO `vec-drop`** — that is the entire leak.
       (`ys` = `(list 10 20 30)` is a #4270-immortal constant → its drop no-ops → only the runtime-built `xs`
       leaks in the pinned case.) ⇒ **fix: emit the SAME `vec-drop` at the empty-match exit arm that the
       sibling recursive arm already emits, for each list operand not referenced by the returned expr.**
     - **SOUNDNESS BY PARITY (self-sufficient for ZIP — no new v-runtime fact needed):** the recursive arm
       ALREADY `vec-drop`s `xs`/`ys` after reading their heads, so they are provably uniquely-owned-droppable
       in this function (each frame owns its `xs` = the prior `xt`; top frame owns the freshly-built list).
       Emitting the identical drop at the exit arm is symmetric-safe. The escape gate (`binding_escapes`)
       guards the general case (a returned expr that DOES reference the operand keeps its drop suppressed).
       mts1's borrow-into-`m` question is DISTINCT and still needs v-runtime; ZIP does not.
     - **v-runtime DEFINITIVE (2026-08-27d, lib.rs cites):** `vec-drop`(handle,i32) = op_vec_split (L5693):
       CONSUMES the vec, frees the boundary spine, returns the OWNED rc-1 TAIL — so the recursive arm's
       `vec-drop(xs,1)`/`vec-drop(ys,1)` prove `xs`/`ys` are OWNED loop-carried operands. `vec-get` (L5042)
       head = BORROW (no dup). RULE: **exit-drop is SOUND only for OWNED binders** (the vec operand / split
       tail), UNSOUND for a borrowed head (UAF into the parent vec). ZIP operands are owned ⇒ sound.
     - **EMIT-SPEC (pinned, awaiting v-runtime soundness sign-off before code):** `zip-sum` compiles to a
       self-tail-LOOP (`func 16`); `xs`/`ys` are loop-carried, reassigned each iter to their split tails; the
       two empty-match arms EXIT the loop returning `acc` and drop NEITHER surviving operand. Fix = at a
       loop-exit match arm, emit a FULL `drop` (heap op 7, cascades + skips immortal children per lib.rs
       L4229) of each loop-carried OWNED heap operand NOT referenced by the returned expr. Gate = owned
       (same classification that made the sibling recursive arm drop it) ∩ `binding_escapes`==false. This is
       a general Perceus drop-placement gap: an operand consumed in one match arm but not a sibling arm is
       not dropped in the non-consuming arm. Acceptance = ZIP/UNZIP/matmul → 0 on `052KQzQP` + value-wrong-
       grep + flap-detect + 0-regress local gate. HIGH-UAF-RISK area (loop-carried × match-arm drop) ⇒
       circulate-before-emit per leak-beats-UAF.
     - **SEAM LOCATED (2026-08-27d, select.rs — machinery ALREADY EXISTS, ZIP escapes it).** The loop-exit
       owned-heap-param drop is `looped_owned_param_drops` (L3757) gated by `param_only_borrowed_or_backedge`
       (L6132, NARROW default-DENY whitelist) + emitted in `select_body`. It fires TODAY only for an
       IDENTITY-carried owned param (same handle every iteration, dropped once at exit). **ZIP's `xs`/`ys` are
       VARYING-owned** — each iteration the slot is reassigned to the split TAIL (`xt`), the previous tail
       consumed by that `vec-split`. The code explicitly DECLINES varying params (L3825 "varying across a
       back-edge — a single exit drop would be wrong"; L5759) — this is precisely the "general owned-heap-param
       pass … documented follow-up, not landed here" (L6146). So the fix is NOT new machinery: it is a NARROW
       WIDENING admitting a varying param whose EVERY back-edge reassignment is a CONSUMING SPLIT of itself
       (`xt` from a `MatchList`/`vec-split` of the same param), for which the current slot value at exit is
       owned + the prior value was consumed ⇒ a single exit-arm drop of the current slot is sound. Widen the
       whitelist's `Call` back-edge arm (L6186) to accept a split-tail arg (not just `Param` identity), and
       have `looped_owned_param_drops`/L3825 admit that varying class. STILL default-DENY: any reassignment
       that is a re-box / mutate-in-place / share (not a consuming self-split) keeps declining (leak, never
       UAF). Circulate this widening to v-runtime for sign-off BEFORE landing (they own the ownership model
       and confirmed the owned-tail-drop soundness).
     - **mts1 RESOLVED — STAYS FENCED (retire the mts1 reclaim sub-arc).** v-runtime definitive: `Map.lookup`
       (op_map_lookup L7826) + the `Some p` unwrap (L1486) are BOTH uniform BORROWS (no dup) — `p` aliases
       `m`'s live CHAMP [k,v] storage. A plain drop of `p` = UAF into `m` (threaded to resume/Map.insert).
       The tick-27a fence was CORRECT. The leak-2 is NOT `p` (nothing owned to reclaim from it) — it is the
       rebuilt `pair` (consumed by BOTH `resume` AND `Map.insert` = a two-consumer thread needing a dup) or
       the `Some` wrapper node, and `pair` is resume-threaded ⇒ 2c.3-adjacent (Core-invisible, fenced). mts1
       is NOT reclaimable in-lane; drop it from the active list. (v-runtime offered a `map-lookup-owned` op
       that dups before returning — declined: it would only relocate the dup, not remove the resume-thread.)
3. Site A, spine-cell reclaim on the back-edge over the fold/count family (pending v-runtime's spine-slot
   + `code.dup_sites` count out of `emit_loop_iteration`).

Each increment lands as a gh PR (base origin/main, admin-merge on 0-regressed LOCAL gate + the rc-leak
probe family — CI is starved, local green is merge-truth), reviewed by v-runtime against the acceptance/
fence set on the debug runtime `052KQzQP` before merge, with the marker flip in the same PR (per v-nix).
