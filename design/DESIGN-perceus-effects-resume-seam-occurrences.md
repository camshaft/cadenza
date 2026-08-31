# Perceus OccTable — the effects RESUME-SEAM occurrences (v-effects slice)

Status: DESIGN — SOUNDNESS-COMPLETE, awaiting Increment B (v-effects, 2026-08-30; rev.4: the §2.1
double-handle residual CLOSED-by-construction per v-memory-safety's co-verification; rev.5 2026-08-31:
LANE FIX — Increment A/B are v-MEMORY-SAFETY's, not v-core-opt's, per v-core-opt's ruling: the dup_at /
dup_sites multiset + `collect_captured_escape_dup_sites` live in `backend/wasm/select/reclaim.rs` =
Perceus dup/drop PLACEMENT, which v-core-opt's charter cedes to v-memory-safety; v-core-opt does only
backend-independent Core OPTIMIZATIONS (CSE/LICM/fold/DCE)). Gate status:
v-memory-safety's Increment A (hczm capture-escape UAF) LANDED #5926; Increment B (dup_at repr) is next;
this slice's two entry points wire into Increment C once A+B are both in. The resume-seam slice of
the uniform per-occurrence reclaim architecture — read
[`DESIGN-perceus-per-occurrence-dup-placement-uniform.md`](DESIGN-perceus-per-occurrence-dup-placement-uniform.md)
first; that doc lists v-effects as CO on the boundary / **resume seams**. This enumerates the
resume-seam heap values *as the POST-SPLICE Core `select.rs` actually sees them*, the `ValueKey`
each maps to, and the borrow/consume class — so `classify_occurrences` can pick them up uniformly.

Owner (soundness/acceptance) + Executor (reclaim.rs dup/drop placement, Increments A/B): v-memory-safety.
(v-core-opt does backend-independent Core opts — CSE/LICM/fold/DCE — NOT reclaim/dup-site placement.)
This slice (resume-seam classification + witnesses): v-effects. Slots into #5857 **Increment C**
(unify), behind v-memory-safety's Increment A (hczm) + B (dup_at repr).

Leverage (concierge, corrected): this unblocks v-effects' OWN cross-function / non-tail resume
increment (the "later increment" the specializer-floor declines cite) — NOT the compiler-ml shred
(that lever moved to v-cadenza-backend solo).

---

## 0.5 CURRENT FILE LOCATIONS (post effects-split + lower→select move, verified 2026-08-30)

⚠️ The monolithic `effects.rs` was SPLIT into `effects/{mod,reduce,thread}.rs`, and the §5 loop-reclaim
predicates live in `backend/wasm/select.rs` (NOT `lower.rs`). Every `effects.rs:NNNN` / `lower.rs:NNNN`
line ref BELOW (§0–§6) is PRE-SPLIT and stale — use this map when wiring the impl once Increment B lands:

- `reduce_handle` → **effects/reduce.rs:14** (pub) · its None-decline path → effects/thread.rs:628
- escaping-continuation reify `k = (fn (#kv) C)` → **effects/reduce.rs:549-554** (was effects.rs:2701)
- `splice_context` → **effects/mod.rs:6818** (was :11762) · `rewrite_resume_to_context` → **effects/mod.rs:6511** (was :11455)
- `build_value_state_tuple` → **effects/mod.rs:2476** (was :5588) · `peel_tuple_value_state` → **effects/mod.rs:5556** (was :10500)
- `param_apply_extra_handled` → **effects/mod.rs:1272** · `thread` → **effects/thread.rs:167** · `thread_bounded` → **effects/thread.rs:492** · `thread_returning_tuple` → thread.rs:19
- §5 predicates (all `backend/wasm/select.rs`): `looped_owned_param_drops` → **:831** · `param_only_borrowed_or_backedge` call → **:901** · `invalidate_varying_params` → **:880**
- the `base#eff<n>` bodyless-spec decline (the cross-fn/non-tail-resume "later increment" this slice unblocks) → **resolve.rs:619-632** (was :622)
- Increment A (hczm capture-escape) LANDED #5926 (rcdzc/core-opt per-occurrence capture escape-dup); Increment B (dup_at repr) STILL NOT landed as of 2026-08-30 (recent reclaim = perf memoization #6140/#6168, not the repr) → this slice stays gated.

## 0. The framing: `resume` is spliced away BEFORE Core

`resume` is SPLICED AWAY in `reduce_handle`/`splice_context` (effects.rs:11762) before Core — there
is NO resume node in `select.rs`. `classify_occurrences` sees the POST-SPLICE Core: the continuation
context inlined at the perform hole. Today it can't tell a resume-thread from a pure consume (both
route via `Core::Call`/`CallClosure`), so it conservatively HOLDS (leak — correct). This slice's job
is to make the resume-seam occurrences CLASSIFIABLE in that post-splice Core. So the enumeration
below is keyed to the Core node each value BECOMES, not to a "resume site" that no longer exists.

## 1. The two splice shapes (what the fold emits)

The fold reifies the continuation two ways (effects.rs ~2686–2758):

- **TAIL-RESUMPTIVE** (`rewrite_resume_to_context`, effects.rs:11455): `(resume v s)` → `C[v]` — the
  delimited continuation `C` (handle body, perform-hole→v) is inlined DIRECTLY at the site. No
  closure. The threaded state `s` and value `v` become ordinary Core in the inlined body.
- **ESCAPING** (`k = (fn (#kv) C)`, effects.rs:2701): when the continuation escapes, it is reified as
  a lambda `(fn (#kv) splice_context(body, perform, #kv))`, lowered to a `Core::Closure` and applied
  at each `(k v)` via `Core::CallClosure`. `C`'s free names (incl. the threaded state) become the
  closure ENV.

## 2. POST-SPLICE occurrence → Core node → ValueKey

| resume-seam value | POST-SPLICE Core node (`select.rs` sees) | ValueKey | note |
|---|---|---|---|
| threaded state `s`, tail-resumptive | `Core::Param{s}` — the self-recursive dispatch-loop state param, threaded on the back-edge | **§5's domain — EXCLUDED from this slice** (see §5.1) | it is a loop back-edge param `looped_owned_param_drops` already models; this slice must NOT dup-each it |
| threaded state `s`, escaping continuation | a slot of the reified `Core::Closure` env; read inside the closure body | `Captured(i)` | closure-env-owned ⇒ `init = 0` (borrowed) — the borrowed-N rule §2.3 |
| the `#st` value/state tuple | a materialized Core tuple from `build_value_state_tuple` (effects.rs:5588), unpacked by tuple-`Proj` per dispatch | `PayloadNode(proj)` | same shape as a sum-payload extraction the reference doc already models — "re-projected per dispatch" |
| resume value `v` | an operand occurrence spliced into `C` (tail), or a `CallClosure` arg (escaping) | (the enclosing value's occurrence) | NOT a new key — an occurrence of an existing value |

NO new `ValueKey` variant. This slice is TWO entry points: **capture-keyed** (escaping-continuation
closure env) and **payload-node-keyed** (the `#st` tuple unpack). The **tail-resumptive back-edge
state param is NOT in this slice** — it is §5's loop-reclaim domain (see §5.1). This is the
resolution of v-memory-safety's mandatory double-handle gate.

## 2.1 §5.1 — the tail-resumptive state param is §5's, NOT this slice (v-mem's mandatory gate)

v-memory-safety flagged: if this slice borrowed-DUPS the tail-resumptive loop-threaded state param
AND §5's `looped_owned_param_drops` owned-DROPS the same param, the two reclaim models double-handle
one value → not a leak but a **double-free** (borrowed-dup expects the loop to keep the ref; §5's
owned-drop frees it → a freed-then-dup'd cell = UAF).

Resolution (verified against `looped_owned_param_drops`, select.rs:4749): **partition by LOCUS — the
tail-resumptive back-edge state param stays §5's; this slice owns only the escaping-continuation
`Captured` slots + the `#st` `PayloadNode` reads.** They are disjoint Core loci (a closure-env
capture / a per-dispatch tuple projection vs. a self-recursive loop's back-edge param), so no param
is handled by both. Grounding:

- §5 `looped_owned_param_drops` targets a PLAIN self-recursive (single-member) loop's heap param that
  is INVARIANT (identity-passed on every back-edge) AND `param_only_borrowed_or_backedge`. It
  owned-drops that param once at loop exit.
- A VARYING loop-carried state (the common effects case — the state ADVANCES, `(Run k)` → `(Run (+ k
  v))`) is ALREADY EXCLUDED by §5 (`invalidate_varying_params`, select.rs:4816 "a varying heap param
  is left to leak"). So §5 neither owns nor drops it — and neither does this slice. It stays a §5
  leak (a separate reclaim gap, NOT this slice's target).
- An INVARIANT state (resume-unchanged, `(resume v s)` threading `s` identity) IS §5's owned-drop
  target. This slice must therefore NOT classify that back-edge thread as a Consume-to-dup — it is a
  back-edge, §5's. That is exactly what the LOCUS PARTITION enforces: the back-edge param is not a
  `Captured` slot nor a `#st` `PayloadNode`, so this slice's two entry points never see it.

**✅ CLOSED BY CONSTRUCTION (v-memory-safety co-verified, rev.4).** The residual I flagged — "IF a
lowering ALSO materializes the loop state into a closure capture, that path is a double-handle
candidate" — is SELF-PREVENTING; no emit-hold, no empirical v-core-opt confirmation needed. §5's
`looped_owned_param_drops` owns-drops a param ONLY IF `param_only_borrowed_or_backedge` returns true
(select.rs:4833, else default-deny at 4835). That predicate (7356-7426) whitelists only borrow
positions / identity back-edge tail-call args / borrow-ops / SumPayload / Match-scrutinee / If-Let
sub-positions; its final arm is `_ => false` (7423-7425) — **a `Core::Closure` capturing the param is
NOT whitelisted → `param_only_borrowed_or_backedge = false` → §5 does NOT own-drop it.** And the
escaping-continuation reification (effects.rs:2701, `k = (fn (#kv) C)` applied via `CallClosure`) IS
exactly a `Core::Closure` capturing the state. So the two cases are exhaustive and disjoint:

- loop state ONLY a back-edge `Param` (never captured) → §5 owns-drops it; this slice never surfaces a
  bare back-edge `Param` → disjoint.
- loop state ALSO captured into the continuation `Closure` (the "residual" path) → the Closure use
  makes `param_only_borrowed_or_backedge = false` → §5 BACKS OFF (leaves it) → this slice's
  `Captured`-dup (init=0, §2.3) is the sole owner → NO double-free, NO explicit exclusion.

Either way §5's default-deny detects the Closure capture and yields ownership to this slice. **The two
reclaim models can never double-handle the same param.** The #4635 double-handle detector (§6) stays
as a belt-and-suspenders regression pin (cheap; witnesses the invariant across future emit changes),
NOT a blocker.

## 3. Borrow vs Consume at each post-splice node (the "reuse, don't re-derive" gate)

The judgments `binding_escapes_dup_aware` must produce per post-splice node; v-mem's gate (2) is to
VERIFY the existing arm logic already yields these, so NO new predicate is added:

- **threaded state, tail-resumptive** — a `Core::Param`/`LocalRef` READ in `C` to compute a value is a
  **Borrow**; the occurrence THREADED into the next dispatch's `#st` tuple (a ctor element / call arg)
  is a **Consume**. `binding_escapes_dup_aware`'s existing `Param`/`LocalRef` arms should classify
  these correctly (read = borrow, ctor-element/arg = consume). *Verify.*
- **escaping-continuation captured state** — a `Captured` slot read inside the closure body: a
  `Proj`-to-scalar / match-dispatch is **Borrow**; a ctor-element / onward-call arg is **Consume**.
  `init = 0` (closure env owns + drops the original), so EACH consuming/escaping occurrence gets `+1
  dup` (borrowed rule). *Verify the `Captured` arm classifies the onward-flow as Consume.*
- **`#st` tuple `Proj`** — the projection extracting `value`/`state` is the standard payload-node
  occurrence; borrow/consume by how the projected value is then used (identical to sum-payload).

**FLAG (the one place a targeted arm may be needed):** a state read that is threaded onward UNCHANGED
(pure pass-through, `(resume … s)` with `s` also read) has TWO occurrences — one Borrow (the read),
one Consume (the thread). If the existing walk collapses these (counts the pass-through as a single
borrow), the Consume is missed → under-dup → UAF. This is the seam to check first against
`binding_escapes_dup_aware`; if it mis-counts, a targeted `Param`/`Captured` consume-on-thread arm is
the minimal fix (NOT a new global predicate).

**✅ §3 FLAG RESOLVED — VERIFIED against `select/reclaim.rs` 2026-08-30 (tick a69), post-split.** The
FLAG's "collapses two occurrences" fear does NOT materialize for the `Captured` entry point, because
Increment A's `collect_captured_escape_dup_sites` (reclaim.rs:1478-1517, LANDED #5926) already runs
PER-OCCURRENCE and PER-NODE: it collects every `Core::Captured{index}` occurrence (grouped by slot),
pre-gates on `capture_escapes_via_body(index)`, then for each occurrence calls
`binding_escapes_dup_aware(db, body, EscapeTarget::Node(occ), false, None)` (line 1512) and inserts
into the dup-site set ONLY the occurrences that escape. So a capture BOTH projected (borrow) AND
escaped (consume) — the hczm2 shape, structurally identical to my pass-through FLAG — dups ONLY the
escaping occurrence; the borrow read is left un-dup'd. No under-dup, no over-dup, NO targeted arm
needed. The `Core::Captured` arm itself (reclaim.rs:188-192) classifies borrow-vs-escape exactly as §3
requires: escapes UNLESS `tail_borrowed` (a `Proj`-scalar / `*.len` / borrow-op ancestor).

**⇒ Increment-B dependency NARROWS.** The code's own comment (reclaim.rs:1493-1495) states the ONLY
case the per-node collector can't yet serve is "a single `Core::Captured` node CONSUMED TWICE" (the
multiset repr — literally Increment B). But the escaping-continuation reify COPIES the continuation `C`
per resume (a multi-shot arm splices a FRESH `C` per `(k v)`, reduce.rs:606-614 / `rewrite_resume_to_
context`), so each resume's captured-state read is a DISTINCT `Core::Captured` node — the
distinct-node multi-escape case Increment A ALREADY handles. **HYPOTHESIS (empirical census pending a
free gate): my §2 escaping-continuation `Captured` entry point may need NO new classifier code — the
effects-reified closures may already flow through `collect_captured_escape_dup_sites` and get correct
per-escape dups under Increment A.** The genuine Increment-B residual for this slice is only a SHARED
`Core::Captured` node consumed twice within ONE continuation copy. To confirm: run an escaping-k effects
case (`crn1`/`(use-k k)` shape) under `--live-objects` and check for 0 (already-reclaimed) vs a leak.
This narrows what I owe v-memory-safety's inventory — flag it as "partially Increment-A-served, census
pending" rather than wholesale "resume-seam-pending gated on B" once censused.

**✅ SECOND ENTRY POINT (`#st` `PayloadNode`) ALSO VERIFIED — a70, symmetric with the Captured one.**
The `#st` value/state tuple (built by `build_value_state_tuple`, effects/mod.rs:2476, unpacked by
tuple-`Proj` per dispatch) lowers to `Core::Proj`/`Core::SumPayload` occurrences that the EXISTING
per-node collector `collect_consuming_payload_sites_expr`/`_cont` (reclaim.rs:786-840) already walks BY
NODE ID: it collects every such occurrence rooted at the scrutinee, classifies each per consuming
position (`consuming` flag, same borrow/consume verdict as `sum_payload_child_escapes_expr`), and
inserts ONLY the consuming compound-heap-leaf sites into `dup_sites` (the snowflake `EscapeTarget::Node`
SumPayload-escape, #5833; memoized #6168). So — exactly like the Captured entry point — the `PayloadNode`
occurrences get correct per-escape dups under the existing machinery for the DISTINCT-node case; the only
Increment-B-gated residual is a SINGLE `Proj`/`SumPayload` node consumed twice. **Net: design Next-step #2
is COMPLETE for BOTH §2 entry points — neither needs a new classifier arm; both are served per-node
today, and the genuine Increment-B need for the whole slice is uniformly just the shared-node-consumed-
twice multiset.** Impl reduces to: confirm the effects-reified Core actually routes through these two
collectors (the empirical `--live-objects` census), then wire only the shared-node residual behind B.

## 4. glb1 = the §2.3 borrowed-N-escapes acceptance case

glb1 (the collapse-continuation join-point duplication) in post-splice Core is a `Core::CallClosure`
(or an inlined `C`) reachable via a JOIN that duplicates the reified continuation across N paths. Its
captured `#st` state is a `Captured` value, `init = 0` (borrowed). The join → N reachable escaping
occurrences of that capture ⇒ **§2.3 borrowed rule: N dups, PATH-AWARE** (mutually-exclusive collapse
arms take MAX-over-arms, not sum — the `ifcap` anti-over-dup: a continuation reached via both arms
needs ONE dup on the taken path). So the OccTable SUBSUMES glb1 — no separate glb1-emit lowering; the
parked glb1 pairing folds away. This is the acceptance proof that the resume seam is "just values
with occurrences."

## 5. Soundness gates (v-memory-safety owns these; captured here so the slice honors them)

- **(a) Conservative — uncertain ⇒ HOLD.** A post-splice seam-node whose classification is uncertain
  stays HOLD (leak), never a speculative drop. Leak beats UAF.
- **(b) The xar5-class forward-escape is a CORRECT must-HOLD, EXCLUDED from get-to-0.** A captured
  value that escapes FORWARD inside the continuation (passed onward within `C`, Core-invisible as a
  resume-forward) must NOT be dropped — dropping = double-free. It stays a leak BY DESIGN (v-mem's
  gate). So the resume-seam get-to-0 target EXCLUDES the resume-continuation-forward-escape minority
  (xar5/xas1 class); those remain intentional holds, not failures.
- **(c) EXIT-SEAM placement, fires once.** Resume-seam drops go at the `reduce_handle` top-level exit
  seam (fires ONCE), NOT the shared per-call/per-dispatch arm — a per-call drop under a loop is a
  double-free. Same terminal-only discipline as the Class-B / terminal-shell reclaim.

## 6. Acceptance witnesses (the resume-seam corpus for Increment C)

- `rsh1` (#5829) + the same-effect straddling twins (heap-state #5883, compound-resume `crn1` #5885)
  — nested-handler state + compound-resume threading, probed sound (a45/a46): PASS-value +
  `live-objects 0` under the OccTable.
- `#5090` flagship-reducer UAF fences + SITE-A over-retention (`#5142`): stay 0.
- `glb1`: flips from parked-leak to PASS `live-objects 0` under the borrowed-N-escapes rule (the
  acceptance proof).
- Path-aware control: a continuation escaping via BOTH collapse arms → ONE dup (the resume-seam
  `ifcap1` analogue) — proves no over-dup leak.
- xar5/xas1 (forward-escape): stay HELD (leak), assert NO speculative drop (gate b) — a drop here is
  the double-free regression to guard against.

## 7. Next steps

1. v-memory-safety soundness-reviews §2 (ValueKey mapping) + §3 (borrow/consume verdicts, esp. the
   §3 FLAG pass-through seam) before any emit.
2. ✅ DONE (a69): v-effects verified §3 against `binding_escapes_dup_aware` (`select/reclaim.rs` post-
   split) — the `Captured` arm + per-node `collect_captured_escape_dup_sites` already classify + dup
   the escaping occurrence only; NO targeted arm needed (see §3 FLAG RESOLVED). glb1's §4 `Captured`
   shape confirmed. Residual: EMPIRICAL census (escaping-k case under `--live-objects`) to confirm the
   reified closures already flow through the Increment-A collector — pending a free gate slot.
3. v-effects implements the §2 entry points into `classify_occurrences` + the glb1 acceptance
   case, feeding v-memory-safety a classified slice — NOT touching `reclaim.rs` dup/drop placement. NOTE the
   narrowed gate: the `Captured` entry point may be Increment-A-served ALREADY (distinct-node multi-
   escape); the genuine Increment-B need is only the SHARED-node-consumed-twice residual + the `#st`
   `PayloadNode` multiset. Lands as part of #5857 Increment C.
