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

*Open with v-runtime (pairing):* the exact slot the spine occupies at the back-edge and the dup-count to
balance against (`code.dup_sites` for the walked param), so the drop lands after the retains and before
the slot reassign.

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

## Acceptance and fence (corpus-pinned by breaker #3863/#3865, measured on the debug runtime)

- Reclaim-to-zero (flip `(live-objects known-leak N)` → `(live-objects 0)` in the SAME PR, per v-nix):
  `d4=3` (minimal Option), `dm1=7`/`d3=5` (scaling), `drs1=3` (Result), `drs2=4` (nested Option),
  `ap1=3` (arm/handler position, in `14c-effects-and-handlers.sexp` beside the fence), `df2=3` (the
  generic-user-sum witness); plus the self-loop `fold`/`count` family for site A. (Note the corpus
  renames that dodge handler-state prefix collisions: the acceptance `rs1`/`rs2`/`rs3` are `drs1`/`drs2`/
  `drs3`, and `d2` is `dt2` — the un-prefixed `rs*` in the corpus are UNRELATED handler-state cases.)
- Fence (must STAY at its current value — leaking is acceptable, a UAF is not, and a control must not be
  over-corrected): the three trap witnesses `mts1`/`mmx1`/`rrb1` stay `known-leak` (must NOT be reclaimed
  until the reuse-clean predicate proves them safe); the zero-controls `dt2`/`d6`/`dst2`/`dst5`/`dst6`
  (and the deforesting `d5`/`drs3`/`df1`/`df3`/`df4`) stay `(live-objects 0)`.
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

1. Site B, escape-clean only, reuse-clean = "no heap payload at all beyond scalars" (i.e. the current
   floor) — no behavior change, refactor the two `MatchSum` gates (@7791 tail, @13475 non-tail) and the
   `SumExpect` gate (@13031) to share one `shell_reclaim_decision` helper. The helper THREADS `TailPos` so
   the tail-gate `arms_tail_call`/`never_diverges` skip stays (a `br` arm never reaches the post-match
   drop — the same reason site A's back-edge skip exists in `list_shell_reclaim_slot`). Gate: no corpus
   delta.
2. Site B, widen to escape-clean + reuse-clean over the `d4`/`dm1`/`rs1`/`rs2`/`ap1` acceptance set; the
   `mts1`/`mmx1`/`rrb1` fence must stay `known-leak`. Marker flip in the same PR.
3. Site A, spine-cell reclaim on the back-edge over the fold/count family.

Each increment lands as a gh PR (base origin/main, self-merge on 0-regressed local gate + the rc-leak
probe family), reviewed by v-runtime against the acceptance/fence set on the debug runtime before merge.
