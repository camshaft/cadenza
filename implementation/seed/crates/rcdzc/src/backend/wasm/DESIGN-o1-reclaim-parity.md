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

## 6. Landed status + remaining follow-on cluster (2026-09-20)

### LANDED to live-objects 0 (all guarded-all GREEN)
- **266** (SITE-A env-cell reclaim for escaped+directly-applied closures) — #9423. Part-1 = `Core::CallClosure`
  classified `Owned` in `heap_operand_ownership` (v-core-opt ownership co-verify); part-2 = the SITE-A env-cell
  drop. 09-functions:266 → 0.
- **465(b)** view-shell husk-only reclaim — #9430. `matchsum_view_shell_reclaim_ok` disjunct
  (`is_owned_single_view_producer ∧ consuming.len()==1 ∧ view_escapes_as_arm_result ∧ owned_compound_boxed`)
  admits the single-consume escaping-view inner shell; the emit dup is `owned_compound_boxed`'s
  `collect_consuming_payload_sites_cont` (same set the gate reads → dup ⟺ gate BY CONSTRUCTION).
  **StrAt fence**: the disjunct is gated on `owned_compound_boxed` (⟹ `Owned`) precisely because
  `is_owned_single_view_producer` also matches `StrAt` (NOT `Owned`) — an un-Owned StrAt escaping view would
  get a shell-drop-without-dup = UAF. Caught pre-land; fenced; end-to-end tripwire = #9431 (13-strings,
  `known-leak`). 10-bytes:727 → 0.
- **5786(a)** invariant-base-consume-reused exit deep-drop — #9432. `param_consumed_reused_in_loop_body`
  (v-mem's inert `allow_base_consume_reduced` flag+arm + my wrapper: the walk with the flag + the
  `varying_param_epilogue_droppable` no-heap-child-escape fences); v-mem's parallel branch at
  `looped_owned_param_drops` fires the exit drop iff `invariant.contains ∧ wrapper ∧
  looped_invariant_param_caller_owned`. 05-compound:5786 → 0.

### REMAINING follow-on cluster (v-core-opt owns; each leak-over-UAF + guarded-all gated)
1. **Interior-view-CHAIN source-transfer** — 465-nested + 1811/1860/2500 (multi-level slice-of-slice).
   The nested OUTER shell is a CORRECT leak-over-UAF residual: the runtime `op_bytes_slice` COLLAPSES
   slice-of-slice (`i` retains rope, not `outer`; bytes_string.rs:326-342), but the COMPILER cannot statically
   assume `outer` is a slice at a general site, so its conservative model treats `i` as retaining its immediate
   source `outer` and keeps `outer` alive → the outer shell can't be husk-dropped without dangling `i`. Path to
   0: teach the reclaim to PROVE `outer` is a slice and re-root `i`'s retained ref `outer→rope`, making `outer`
   surplus. Needs a static slice-chain proof (design-first).
2. **StrAt-escaping → 0** — flip #9431 `known-leak`. `StrAt` is not `Owned`, so `owned_compound_boxed` doesn't
   dup its escaping view → the 465(b) fence correctly DECLINES it (leak). Path to 0: a StrAt-escaping-specific
   child-dup co-placed with the husk-drop (co-design w/ v-mem placement). On delivery, flip #9431 → 0
   (value stays 3111/3122/3144/3100).
3. **5890 SumPayload-base variant** — `(List.push (match bx ((B xs) xs)) 99)`: `xs` is a Sum-EXTRACTED child,
   not a direct base, so `arg_reclaims_binder_as_base` (direct-base only) declines it. Path to 0: extend the
   base-consume recognition through a SumPayload extraction, with aliasing care (`xs` aliases `bx` — the exit
   drop must not free a child still reachable via `bx`). Likely a co-design.

All three admit only under §3 (fresh-owned / proven-non-aliasing / source-chain-balanced); guarded-all
(`gate-local`), NOT `coarse-<chapter>`, is the authoritative pre-land gate for every slice (chor-driver #9413:
a reclaim UAF surfaces in a DIFFERENT chapter than the one changed).
