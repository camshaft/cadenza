# DESIGN: O1-reclaim-parity (close the O1-opt-gating + interior-view leak class)

**Status:** DESIGN (operator-greenlit 2026-09-20 via concierge seq-relay of the 5620 scope call —
"close out on perfect memory safety"; authorized to proceed, sequence as capacity allows).
**Owners:** v-core-opt (escape-classification / consuming-analysis / admit conditions) +
v-memory-safety (shell-drop PLACEMENT / O1 Let-epilogue + loop/varying reclaim / census + pin flips).
**Gate:** `cargo xtask fleet gate-local` (guarded-all) — NOT `coarse-<chapter>` alone. A reclaim
regression (leak OR UAF) surfaces in a DIFFERENT chapter than the one changed (the chor-driver UAF
from #9413 only showed in cad-test-choreography, never in the 28-wit/06 gates the change targeted).

## 1. Problem

The B2 sharing-aware-emit pass (`opt::run_sharing_aware_emit`) binds a shared heap node into a
`Core::Let` so the emit-analysis stops re-descending it. It is O2-gated for wasm because the O1 emit
does NOT insert the reclaim the O2 pipeline does for a bound / shell-reclaimed shared node. Two
consequences, both currently held as tracked known-leak pins (leak-over-UAF, O2/O3-clean):

- **O1-opt-gating leak class** (5620 `fadd` straight-line product shares + siblings): the shared
  2×-read arg tuples never become reclaimable Let-bindings at the default O1. B2 reclaims them at O2.
  A naive "run B2 at O1" attempt (reverted) reclaimed 5620 but introduced a coarse-05 rotate-by-k UAF
  (a shared list bound into a Let double-freed across two consuming list-ops) + a checked-mul sum-shell
  leak — i.e. the O1 emit lacks the loop/varying + sum-shell reclaim parity.
- **Interior-view cluster** (10-bytes 465/727/1811/1860/2500, 702, 3040): a `Some(view)` inner match
  whose `Bytes.slice`/`String.slice` view payload escapes (dup'd) leaves the inner Option shell husk
  unreclaimed; `matchsum_view_shell_reclaim_ok` deliberately declines the escaping-heap arm (leak-over-
  UAF) because the view ALIASES its source chain (outer→rope) and the shell deep-drop could under-drain
  it. Same hazard from the consumed-param side: v-mem's 05-compound:5786 (a consumed invariant param the
  caller reuses).

## 2. Sub-problems

### (a) B2-bound-node reclaim parity (closes the O1-opt-gating class)
When a shared node is `Core::Let`-bound at O1, the O1 emit must reclaim it exactly as O2 does:
- **Straight-line product share** (5620): the opt-level-independent Let-epilogue (`emit.rs`) already
  drops a non-escaping heap Let-binding — this half works. [Verified: fresh-producer path.]
- **Loop-carried / varying share**: the O1 loop-reclaim omits the drop the O2 pipeline inserts →
  a per-iteration leak or (rotate-by-k) a double-free when the bound node aliases a spine consumed by a
  recursive callee. PARITY NEEDED: O1 loop/varying reclaim must match O2 for a B2-bound node.
- **Sum-shell share** (checked-mul): a bound `Some/None` SumNew shell's husk must be dropped when the
  payload escapes — the O2 shell-reclaim does this; the O1 Let-epilogue treats the whole binding as
  escaping (payload escapes) and skips the husk drop. PARITY NEEDED: sum-shell husk drop at O1.

### (b) View-shell admit condition (closes the interior-view cluster)
For a `Some(view)` inner match whose view escapes dup'd, admit the inner-shell deep-drop ONLY when the
shell-drop provably does NOT under-drain the view's source chain. The escape-dup 1:1-net argument ALONE
is INSUFFICIENT — it died at rotate-by-k and the chor-driver UAF. Requires a **source-chain ownership
proof**: the escaped dup'd view (and the shell-drop cascade) net correctly against every source the
view aliases (outer→rope→…), OR the shell-drop provably does not cascade into the aliased source.

## 3. Safety framework (from this week's two UAFs — #9413 chor-driver + the 5620 rotate-by-k revert)

> **escape-dup does NOT license a shell-drop when the escaped value ALIASES a shared structure.**

Admit a reclaim only when one of:
1. **Fresh-owned producer** — the scrutinee is a freshly-minted owned value (`Core::Call`/`HostCall`/
   `AstDecode`/`StrFromBytes`), so its payload is owned exclusively → no external aliasing. (Already the
   `c0c16c88c4` fix: `nontail_param_compound_extra_ok(bare_payload_result_ok=true)` for that path.)
2. **Proven non-aliasing** — the bound/extracted value provably shares no cell with a structure that
   outlives the drop (no consuming reader of a shared spine; no view into a longer-lived source).
3. **Source-chain balanced** — (view case) the drop cascade nets against the escaped dup'd view across
   the FULL source chain (per real rc-trace semantics, sub-problem (b)).

Everything else stays declined (leak-over-UAF). This is why the general PARAM path keeps the G4 fence.

## 4. Plan / sequencing

1. [v-mem] Produce emitted rc-traces: 465 inner Some(view) shell + view source-chain (outer→rope) drops
   at O1; 5786 consumed-invariant-param reuse. → grounds §2(b) + tells us if (a)/(b) share one primitive.
2. [v-core-opt] Fill the §2(b) admit condition + the §2(a) sum-shell / loop parity conditions against
   the traces. Prove each against the §3 framework.
3. [v-mem] Implement the O1 Let-epilogue + loop/varying + sum-shell reclaim placement; [v-core-opt] the
   escape/admit classification. Land incrementally, each slice gated guarded-all, flipping only the
   pins that slice proves to 0 (never a blanket B2-at-O1 flip — that was the reverted mistake).
4. Close the O1-opt-gating class (5620 + siblings) and the interior-view cluster to live-objects 0.

## 5. Grounded findings (v-mem rc-traces, 2026-09-20; fixed seed compiler O1)

Two DISTINCT admit conditions, one safety framework (§3). **They do NOT share one primitive** (answers
the first open question below). I own both admit classifiers; v-mem owns the reclaim PLACEMENT (the
husk-only drop / loop-exit drop emit) + census + pin flips.

### (b) 465 — inner `Some(view)` shell-husk reclaim  [`/tmp/o1parity-465-rctrace.txt`]
Repro: `pick`/`Bytes.concat` rope → `match (Bytes.slice rope 1 5) Some outer → match (Bytes.slice
outer 1 3) Some i → i`; then `i` feeds value-eq + a `Map.lookup` key. main(0)=17, live-objects 4.
- **What leaks:** ONLY the inner `Some(view)` Option SHELLS — node#10/#11 (first nested-match eval) +
  node#15/#16 (second eval): `ALLOC … no freed DROP`. The payload view `i` is escape-dup'd (node#10/#15
  `DUP 1→2`).
- **Source chain IS reclaimed independently:** node#7 (rope Compound) `DUP 1→2` then `DROP…→0 [freed]`;
  node#12 (outer-slice VIEW) same. So freeing the inner shell does NOT touch a live source.
- **Admit condition (my lane):** admit a SHELL-HUSK-ONLY reclaim of the inner `Some(view)` shell (free
  the shell cell; do NOT deep-drop / cascade into the escaped payload `i`) when: (1) the payload escapes
  as a DUP'd arm result (`view_escapes_as_arm_result` ∧ the escape is a dup site), (2) the shell is
  dead-after (only `i` is used downstream, not the shell), (3) the view's source chain is reclaimed on
  every path independently of this shell. The escape-dup accounts for `i`'s surviving ref; the husk-only
  drop reclaims exactly the un-dropped shell cell. This is why the current DEEP-drop is declined
  (`matchsum_view_shell_reclaim_ok` bails the escaping-heap arm) — a deep-drop cascades into `i` (aliases
  outer→rope) → UAF; a HUSK-ONLY drop does not. **New emit primitive needed: shell-husk-only drop.**

### (a) 5786 — loop-exit invariant-param reclaim  [`/tmp/o1parity-5786-rctrace.txt`]
Repro: `mb` builds `base=[0,1]`; loop threads `base` INVARIANT, each iter `List.len(List.push base 99)`;
`base` reused every iteration and by the caller. main(2)=6, live-objects 2 (CONSTANT across m — not
iteration-scaling).
- **What leaks:** node#1 (base list spine, the invariant param) + node#2 (its boxed wrapper Sum). The
  per-iteration `(List.push base 99)` results ARE reclaimed (node#3/#4, node#5/#6 alloc+freed per iter).
  The residue is the invariant `base` never dropped at loop EXIT.
- **Admit condition (my lane):** at loop exit, drop the consumed-and-reused invariant param spine when
  the caller does not need it after (caller-ownership) — extend `looped_invariant_param_caller_owned`
  (which already fences the CAESAR compare-arm case) to the consumed-arm relax. Safe iff caller discards
  (main does) or dups. No view, no shell-husk. **Reclaim placement: loop-exit drop (v-mem).**

### 266 (SITE-A) — a THIRD, simpler class (non-view, non-invariant)
Handled separately (v-mem drafts). The coupling I already supplied: SumExpect is Owned (payload
independent, SITE-A-droppable) iff `heap_operand_ownership(source)==Owned ∧ !matches!(core_of(source),
BytesSlice|StrSlice|StrAt)` — element/copy producers dup the payload in; view producers alias. Same
element-vs-view discriminator as (b), reused.

## Open questions (fill during co-design)
- **ANSWERED:** (a) and (b) are TWO distinct proofs, not one primitive (v-mem trace 2026-09-20): (a) is
  a whole-spine loop-exit drop gated on caller-ownership; (b) is a shell-husk-only drop gated on
  view-source-chain non-under-drain + payload-escape-dup. Shared only via §3.
- Does the shell-husk-only drop need a NEW Lir/emit op (free-cell-without-cascade), or can it reuse an
  existing husk-drop path? [v-mem placement lane]
- Can the O1 loop-exit invariant-param drop reuse the O2 pipeline's drop-insertion, or a separate
  layout-preserving O1-emit pass (like B2)?

## 6. Reclaim-family map + implementation status (updated 2026-09-21)

The reclaim work resolved into DISTINCT families, each closed by a SPECIFIC discriminator/fence (never a
blanket admit). One unifying discipline (§3): **leak-over-UAF** — when a discriminator can't be proven,
DECLINE the reclaim (a leak, never a double-free); **dup ⟺ drop lockstep** on ONE shared predicate; and
**guarded-all is MANDATORY** before landing any reclaim change (a reclaim UAF surfaces in a DIFFERENT
chapter — the #9413 chor-driver / #9435 tripwire locus; `coarse-<chapter>` alone is insufficient). Census
is read from the NIX debug-counters runtime — native `--report-live-objects` is UNFAITHFUL (census-flaky).

- **F1 — interior-view shell-husk (10-bytes 465b/727).** `matchsum_view_shell_reclaim_ok`'s
  `owned_compound_boxed` disjunct (`compound_boxed && heap_operand_ownership==Owned`) admits a single-consume
  escaping-view husk drop; a StrAt fence blocks the non-Owned view (StrAt aliases its source). LANDED #9430,
  tripwire #9431. Residual: a StrAt-escaping view stays known-leak (needs a StrAt-escaping child-dup).

- **F2 — escaped-child-dup (06-numeric 14929/14941).** A non-tail spine PARAM whose arm BARE-returns a heap
  payload child: escape-dup the child + reclaim the shell. **DOUBLE-FENCED** — (depth) admit only a DIRECT
  single-level `SumPayload{scrutinee}` (`payload_in_result_bare_escape_ok`, excludes json `encode`'s nested
  `r.raw`); (ownership) `body_is_self_recursive` at BOTH coupling sites (collector + G4 relax) — a mutually-
  recursive body on borrowed elements does not own its param. LANDED #9435 + over-drop tripwire #9436.
  Residual: a DEEP-nested escaped child stays known-leak (the interior-view source-transfer follow-on).

- **F3 — loop-exit invariant-param reclaim (05-compound 5786(a)).** A consumed-AND-reused invariant param
  whose per-iter `List.push base` result is scalar-reduced: the loop-EXIT deep-drop reclaims the final
  un-consumed value. `param_consumed_reused_in_loop_body` (walk with `allow_base_consume_reduced`) gated on
  `looped_invariant_param_caller_owned` (the AXIS-A/CAESAR caller-ownership fence). LANDED #9432.

- **F4 — caller-retains invariant-base (05-compound 5786-postloop).** Same base consumed-reused invariant,
  but ALSO read AFTER the loop → the loop-exit drop must DECLINE (caller_owned=FALSE); the CALLER reclaims
  instead. `def_consumes_param` reclassifies a DUP-BACKED base-consume as a BORROW (base occurrence ∈ the
  callee's `dup_sites` ⟺ the op path-copies rc>1, NOT a rc1 FBIP-reuse) → the caller retains + drops at its
  last use. CONTAINED to `def_consumes_param` (no global `param_only_borrowed_or_backedge` edit — the #9423
  blast-radius lesson). The classifier (`c861652be2`) is only load-bearing once CONSULTED at the ACTUAL
  caller-drop decision in `call_arg_caller_drops`, admitted (bypassing its boundary-owned /
  `param_escapes_body` / looped gates) iff FOUR conjuncts: (A) `!def_consumes_param(callee,i)` [back-edge-
  aware borrow]; (B) arg DUP-BACKED (∈ the caller's `dup_sites`); (C) `self_def ∉ mutual_loop_group(callee)`
  [EXTERNAL caller — EXCLUDES self/mutual recursive calls]; (D) non-tail. Conjunct (C) is LOAD-BEARING: a
  first attempt SUPPRESSING the dup at the `mark_binder_dups` `Core::Call` site (instead of adding a caller-
  drop) RED'd guarded-all — it fired on the callee's OWN non-tail self-call args (a recursive fold's
  `(f … arg …)`), where the retain-dup is load-bearing per frame → UAF (06-numeric BigInt `unreachable`,
  guide-0406 OOB); the mark site lacks the caller context to exclude self-calls. TWO MORE conjuncts were
  needed after guarded-all caught two further double-frees (the 4-conjunct admit was insufficient — (A) is
  the GENERAL borrow verdict, not the 5786 specificity, and full `dup_sites` conflates categories): (B′)
  key (B) on `caller_surplus_dup_sites` = the retain-only `dup_sites` MINUS `collect_shell_reclaim_child_dups`
  (05:2199 `fst-sum (P a _b)=(sum a)` double-freed `a`, a shell-reclaim child already balanced by the shell
  drop) + (E) `heap_operand_ownership(arg)==Borrowed` (genuine live-after surplus, excludes last-use moves);
  and (G) yield to the callee loop epilogue — admit only if the callee's `looped_owned_param_drops` does NOT
  contain the param slot (05:3709 `sum-at`, a recursive `List.at` consumer that epilogue-drops `xs`, so the
  caller-drop was a second drop; (G) is the caller-drop ⊕ callee-self-reclaim complementarity, twin of gates
  (6)/(6b)). LANDED #9452 (final admit = the v-core-opt classifier + 5 gates A/B/C/E/G; #9434 (05:5795)
  flipped known-leak 2→0; local-gate + coarse-05 hard-0 + cad-test-json 120/0 + full rcdzc 1544/0 GREEN,
  census 4m+3 correct live-objects 0 for m 0..7 no trap; rebased cleanly onto the #9449 fence). The
  05:5794 caller-BORROWED variant (main does NOT own the base) correctly STAYS known-leak — gate (E)
  Borrowed + (B) caller-surplus decline it (the over-reclaim tripwire, breaker re-fenced). NB v-mem's broader
  caller-drop also reclaims a small bonus tighten pile in coarse-05 (a shared-aggregate projection consumed-
  then-read, a sum-payload child consumed while scrutinee live, a map-returned scalar-payload sum via ctor, a
  wildcard-consumed lookup) — v-mem verifies + flips those in a corpus-only follow-up (their census lane).

- **F5 — SITE-A closure-env-cell reclaim (09-functions 771 family, ~11 cases).** An INVARIANT borrow-clean
  closure loop-PARAM applied per iteration: its per-application caller dup is spurious → drop it per apply
  (`closure_env_invariant_borrow_clean_binders` → the `CallClosure` SITE-A env-drop, gated per-SITE on
  `dup_sites` — only the dup'd application site drops; the entry ref rides the loop-exit `looped_owned_param_
  drops`). THREE fences added after guarded-all caught over-frees: (a) **non-tail-selfcall** — exclude a
  param threaded into a NON-TAIL loop-member self-call (require a PURE self-tail-loop; the `filt` shell-drop
  cascade otherwise overlaps a child frame's reclaim → UAF); (b) **shared-heap-capture dup** at the
  `Core::Closure` build — a SHARED (multi-used) heap capture must be dup'd into the env (json-`encode`-style
  `3485`: an un-dup'd borrowed fn capture is over-freed by the env-drop cascade); (c) **caller-ownership**
  — gate the b‴ Param-admit on `looped_invariant_param_caller_owned`, the SAME AXIS-A all-call-sites-Owned
  fence the loop-exit `looped_owned_param_drops` ALREADY uses (a latent asymmetry omitted it from the
  per-application drop). The b‴ relaxation of the "never Param" rule is unsound for a BOUNDARY-CONSUMED
  closure — an EXPORT/host-resource param threaded in (21-host-closures `iter`'s `g` via the `apply-n`
  export) is a host resource the guest must NOT reclaim; the per-application drop freed it → the next
  iteration read freed env → `unreachable` (a SHIPPED UAF regression from #9440, caught by v-cadenza-ci).
  The recognizer cannot tell a fresh guest closure from a boundary resource on the looped body alone;
  `looped_invariant_param_caller_owned` can — it declines `iter` while admitting `times2`'s fresh Owned
  `(mk-adder k)`. LANDED #9440 (SITE-A + fences (a)/(b) + capture-retain (Y); census main 5→32 live-objects
  0). Fence (c) LANDED #9449 (guarded-all GREEN, CI-reconfirmed: coarse-21 fixed, coarse-11 `1383` preserved
  at 0; the boundary-consumed cases #9440 over-flipped revert to their known-leak baselines). Only the CLEAN
  `1383` case is a flipped win; `3485`/`filt`/`714`/`771`/`798` stay known-leak (leak-over-UAF) — the fences
  + (Y) are load-bearing UAF guards.

- **F6 (queued) — multi-borrow / multi-sibling closure-env residual (09-functions 771, 798).** The SITE-A
  (F5) env-drop flips the SINGLE-application invariant borrow-clean closure loop (09:713 → 0); two harder
  shapes stay known-leak. (i) **multi-apply base (771):** the owned closure `f` is applied k>1 times on the
  base arm (`(+ (f 0) (f 1))`) — k `CallClosure` BORROWS precede the ONE loop-exit env-drop; the captured
  heap env (`xs`) leaks because the reclaim declines when >1 borrow precedes the drop. ADMIT SKETCH (v-core-
  opt): the exit-drop is safe when EVERY base-arm use of `f` is a `CallClosure` BORROW sequenced BEFORE the
  loop-exit drop (all k borrows release before the single drop → no race) — relax the SITE-A single-borrow
  gate to k-borrow-all-borrows. (ii) **two-sibling self-call (798):** `f` passed to two sibling self-calls
  (`(+ (go f …) (go f …))`) forces a dup-per-frame (last-sibling consume-spare) + a base-arm borrow; the dup
  + spare + loop-exit env-drop must BALANCE (an over-drop double-frees the shared closure a sibling still
  holds). HARDER — the sibling consume-spare interacts with the env-dtor; needs an emitted rc-trace to ground
  the balance. Condition = v-core-opt; PLACEMENT = v-mem (blanket auth). ADJACENT to F5 — coordinate, do not
  double-drive.

- **QUEUED:** 14966 (non-tail borrowing-param — invariant borrow-only heap param dup'd per non-tail arg,
  never dropped on unwind); interior-view-chain 1811/1860/2500 (source-transfer: re-root the view's ref
  outer→rope) + F1's StrAt residual + F2's deep-nested residual; 5890 (SumPayload-base variant).

**Ownership note (2026-09-21):** the operator granted v-memory-safety blanket authorization to cross into
the EMIT lane to close this pile out ("zero memory safety issues"); v-core-opt owns the escape/consume
CLASSIFICATION + the admit conditions and serves as the SITE-A/reclaim adversarial-diagnosis expert, v-mem
owns reclaim PLACEMENT + census + guarded-all + pin flips. Coordinate per-family to avoid double-landing.

### Open questions — resolved by the implementation
- (a)/(b) ARE two distinct proofs (confirmed): F3 whole-spine caller-ownership drop vs F1 shell-husk-only
  drop. F4 later split from F3 (post-loop-read → caller-retains, not loop-exit).
- The husk-only / caller-retain drops reused EXISTING emit paths (the loop-exit `op_drop`, the caller
  scope-drop) — no new free-cell-without-cascade Lir op was needed; the fences are all in the ADMIT
  classifiers, keeping the emit layout-preserving.
