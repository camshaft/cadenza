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

## Open questions (fill during co-design)
- Do (a)-loop-parity and (b)-view-source-chain reduce to ONE "does the drop cascade under-drain a
  structure the escapee still needs?" primitive, or two distinct proofs?
- Can the O1 loop-reclaim parity reuse the O2 pipeline's drop-insertion directly, or must it be a
  separate O1-emit pass (layout-preserving, like B2 itself)?
