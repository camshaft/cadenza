# Uniform per-occurrence dup/drop placement — the scalable reclaim architecture

Status: DESIGN (v-memory-safety, 2026-08-29, operator-directed: "the team _has_ to fix this … a design
that can actually scale … whatever architecture they have right now isn't working"). This specifies the
REPLACEMENT for the current escape-dup architecture. It is grounded in the Perceus reference
([`DESIGN-perceus-the-static-bar-algorithm-reference.md`](DESIGN-perceus-the-static-bar-algorithm-reference.md))
— read that first; this doc is HOW we make rcdzc's emit realize it uniformly.

Owner: v-memory-safety (strategy/soundness/acceptance). Executor: v-core-opt (the `select.rs` emit).
Co: v-runtime (rc ops + debug balance assert), v-effects/v-rust-backend (the boundary/resume seams).

---

## 0. The bug that proves the architecture is broken

`hczm1` — a closure capturing a tuple `a` and returning `#tuple(a a)` (the capture ESCAPES in TWO
occurrences), then dropped — **traps `wasm unreachable` on release (an over-free / UAF)**. The single-
escape twin (`hcz1`, `#tuple(a … )` once) is clean. So the defect is MULTIPLICITY: N escaping
occurrences of one owned/borrowed value need N (or N−1) dups, and the current architecture cannot emit
them.

## 1. Why the current architecture cannot scale (root cause, by code)

All in `backend/wasm/select.rs`. Three facts, each fatal:

1. **The escape judgment returns a BOOLEAN, collapsing multiplicity.** `binding_escapes_dup_aware`
   (@1405) walks every occurrence of a binder/capture/payload and returns `bool` — "does *some*
   occurrence escape". It visits each occurrence (the `LocalRef`/`Param`/`Captured` arms @1428/1440/1452)
   but throws away the COUNT. You cannot place per-occurrence dups from a yes/no answer.

2. **Dup-sites are a `HashSet<StructId>` — a set cannot represent "dup this twice".** The collectors
   (`collect_captured_escape_dup_sites` @2678, `collect_sumpayload_escape_dup_sites` @2727) insert an
   occurrence NODE into a set; emit dups a node iff it is present. Membership is 0-or-1; multiplicity is
   unrepresentable by construction.

3. **Multiplicity is explicitly PUNTED, and there are N ad-hoc collectors.**
   `collect_captured_escape_dup_sites` groups occurrences by capture index and does
   `if occs.len() != 1 { continue }` (@2683) — a capture used in >1 occurrence gets **ZERO** dups (the
   hczm1 UAF). Meanwhile the SUM-payload escape is a SEPARATE collector with a SEPARATE predicate, and
   the binder path is a THIRD (`collect_dup_sites`/`mark_binder_dups` @2774/@3362). Each class = a new
   collector + a new escape predicate; the predicates disagree at the edges (this is the source of the
   §7 gap-map leak/UAF classes in the reference doc). The architecture is **O(number of shapes)**, not
   one judgment — so every new boundary/escape shape reopens the class.

The deeper problem underneath all three: rcdzc has **no single owned/borrowed judgment**.
`heap_operand_ownership(Param)` defaults to BORROWED (the inverse of Perceus's owned default), and the
collectors bolt narrow OWNED/escape exceptions on top. Perceus is one rule set applied uniformly; this
is a patchwork approximating it per-shape.

## 2. The scalable architecture: one per-occurrence classification → a dup MULTISET

Replace the N boolean collectors + set with ONE pass that computes, for every heap value in a body, the
**classification of each occurrence**, and derives dup/drop placement from it by the Perceus rules.

### 2.1 The occurrence table (the new core data structure)

```
enum Use { Borrow, Consume { escapes: bool } }   // Borrow = read-without-retain; Consume = ownership out
struct OccTable {
    // per (value-key, occurrence-node) → how this occurrence uses the value
    uses: HashMap<(ValueKey, StructId /*occurrence*/), Use>,
    // derived: how many dups to emit AT this occurrence node (0, 1, … — a COUNT, not a set)
    dup_at: HashMap<StructId, u32>,
    // derived: values needing a reclaim drop, and where (branch-start / let-end / loop-exit)
    drops: Vec<(ValueKey, DropSite)>,
}
enum ValueKey { Param(StructId), Let(StructId), Captured(usize), PayloadNode(StructId) }
```

`ValueKey` unifies the three current collectors' scopes: a `let`/param binder, a closure capture slot,
and a boundary-owned payload-extraction node are all just *values with occurrences*. `dup_at` is a
COUNT keyed by occurrence — this is the one change that makes multiplicity representable.

### 2.2 The classification (reuse the existing engine, return richer data)

`binding_escapes_dup_aware`'s per-occurrence walk (borrow vs consume, escape vs not) is ALREADY correct
— it just discards the result. Refactor it (or wrap it) to POPULATE `OccTable.uses` per occurrence
instead of `||`-ing into a bool. Same borrow/consume classification (a `Proj`-to-scalar / `*.len` /
match-dispatch is a Borrow; a ctor element / call arg / result / tail is a Consume; a nested-compound
`Proj` that transfers a child handle is a Consume-that-escapes). No new predicate — the SAME walk, kept
per-occurrence. This is the single judgment; the three collectors become three ENTRY POINTS
(param-keyed, capture-keyed, payload-node-keyed) that seed the same classifier.

### 2.3 The placement rules (Perceus, uniform — this is the whole fix)

For a heap value `v` with initial owned reference count `init` in this frame (`init = 1` for a fresh/
owned local or an OWNED param; `init = 0` for a BORROWED param or a closure-env-owned capture — the
owner holds the real ref), let its occurrences be classified by 2.2. Then:

- **dups.** Emit `dup v` at each occurrence that needs its own reference and is not covered by the moved
  original:
  - OWNED value (`init = 1`): the LAST consuming occurrence MOVES the original (no dup); every EARLIER
    consuming occurrence gets `+1 dup`. Borrows get none. ⇒ `dup_at` counts = (K_consume − 1) spread
    one-per-consume-before-the-last.
  - BORROWED value (`init = 0`: a borrowed param, or a capture the closure env owns and will drop): EACH
    escaping/consuming occurrence gets `+1 dup` (the owner still drops the original, so every out-flow
    needs its own ref). ⇒ `dup_at` counts = K_escape, one per escaping occurrence. **This is the hczm
    fix, the snowflake fix, and the hcz fix — one rule.** (`#tuple(a a)` over a borrowed capture → 2
    escaping occurrences → 2 dups → the closure-drop cascade nets each returned ref to a live rc.)
- **drops.** A value with a surviving owned reference where it becomes dead gets exactly one `drop`:
  - `smatch` Γi′ — at each branch start, drop the owned vars (incl. the scrutinee once its kept fields
    are dup'd) dead in that branch (reference §2).
  - `sbind-d` / end-of-scope — an owned binding with zero consuming occurrences, or whose consumes are
    all balanced by the owner, drops its surviving ref at its last use / scope end.
  - loop-exit — a loop-carried owned value dead on the exit arm (the ZIP/self-loop class).

**⚠ Occurrences are counted PER CONTROL-FLOW PATH, not by a flat syntactic tally.** Mutually-exclusive
branches (if-arms, match-arms) each run alone, so an occurrence in one arm and an occurrence in a sibling
arm do NOT sum — they contribute the MAX over arms. Only occurrences simultaneously live on the SAME path
sum. Concretely: a captured/owned value `a` escaping via BOTH arms of `(if c a a)` needs ONE dup (the
executed arm consumes one reference), not two; a flat by-occurrence count (2) would OVER-retain on the
taken path → a leak. Witnessed: `ifcap1` `(fn (q) (if (> q 0) a a))` over a captured tuple — under the
current `len != 1` punt it gets ZERO dups → over-free (UAF); under a *naive* per-syntactic-occurrence fix
it would get 2 dups → leak; the correct count is 1. `mark_binder_dups` ALREADY computes this correctly
(its `live_after` / branch-aware model), which is the third reason it is the reference for the unified
count — do NOT re-derive a flat tally over `collect_captured_occurrences`' by-index `Vec`.

**The invariant that makes it sound and checkable** (Perceus §3.4, multiplicity 1): on every control
path, `init + Σ dup_at(v) == Σ consume(v) + Σ drop(v)`. dups exactly cover the consuming/escaping
out-flows the original doesn't; drops cover the survivors. Emit this as a debug balance assert per value
(v-runtime's lane) — a mismatch is a leak (dups > needed) or an over-free (dups < needed) caught at
compile time, not by a corpus census.

## 3. Concrete rcdzc mapping (what changes, incrementally)

1. **Representation.** Replace the `HashSet<StructId> sites` threaded into emit with `dup_at:
   HashMap<StructId, u32>` (count). Emit at an occurrence: `for _ in 0..dup_at[node] { emit OP_DUP }`.
   `OP_DUP` import count follows the same map (keeps emit + import in agreement, as today).
2. **Classifier.** Add `classify_occurrences(db, body) -> OccTable` wrapping the
   `binding_escapes_dup_aware` walk to record `Use` per occurrence. Keep the current borrow/consume
   arm logic verbatim (it is correct) — only stop collapsing to bool.
3. **Unify the collectors.** `collect_captured_escape_dup_sites`, `collect_sumpayload_escape_dup_sites`,
   and the binder path in `collect_dup_sites`/`mark_binder_dups` all become thin entry points that seed
   `classify_occurrences` with the right `ValueKey` and read `dup_at`/`drops` out. Delete the
   `occs.len() != 1` punt (@2683) — multiplicity is now handled by the count.
4. **`init` (owned vs borrowed).** Source `init` from the existing signals: a `db.lifted` boundary-owned
   param and a closure-env-owned capture are `init = 0` (borrowed — owner drops); a fresh owned local /
   fn-owned scrutinee is `init = 1`. This is where the owned/borrowed judgment lives; keep it explicit
   and conservative (unknown → treat as borrowed, i.e. dup every escape — leaks beat UAF, reference §6.4).

## 4. Soundness gates (mine to enforce on every increment)

1. **Static only** — `dup_at`/`drops` are functions of the occurrence classification, never a runtime rc
   (reference §0/§6.1). The dup COUNT is compile-time.
2. **Balanced** — the §2.3 invariant holds per value per path; land the debug balance assert alongside.
3. **Borrow ≠ consume, per occurrence** — reuse the existing arm classification; do NOT re-derive a new
   escape predicate (divergent predicates are the current bug).
4. **Reuse-clean / FBIP** — a shell/scrutinee deep-drop must still not free a cell an FBIP rebuild aliased
   (reference §4); the OccTable does not change that gate, it composes with it.
5. **Leak beats UAF** — unknown `init` or unclassifiable occurrence ⇒ treat as borrowed + dup the escape
   (leak), never skip (UAF).

## 5. Acceptance (corpus, on the debug-counters store + release/CAD)

- **UAF flips:** `hczm1` (multi-escape tuple), `hczm2` (read+escape), a multi-escape LIST and MAP twin →
  PASS value + `live-objects 0` + no release trap.
- **Path-awareness (the anti-over-dup control):** `ifcap1` `(fn (q) (if (> q 0) a a))` (capture escapes via
  both if-arms) → `live-objects 0` (proves ONE dup, not two — a naive per-syntactic-occurrence fix leaks
  here even while `hczm1` reaches 0); `ifcap2` (escape in one arm) stays PASS 0.
- **No regression:** the single-escape family `hcz1`–`hcz5` stays PASS 0; the snowflake `lower` stays CAD
  138/0; the sum-payload boundary cases stay 0; the whole corpus fail-set is ADDITIVE-only (no
  `Todo→Fail`, no value-wrong, no flap — reference §8 tools).
- **Balance assert** green across the corpus (no per-value rc imbalance at compile time).

## 6. Migration (land without a big-bang regression)

- **Increment A (UAF stopgap, land first if the fix can't wait):** in `collect_captured_escape_dup_sites`,
  handle `occs.len() > 1` by emitting one dup per ESCAPING occurrence (not the `!= 1` punt). Narrow,
  fixes hczm1/hczm2, additive. This is the patch — NOT the architecture; do it only to close the live UAF
  fast, then do B.
- **Increment B (representation):** introduce `dup_at: HashMap<_,u32>` behind the existing collectors
  (each populates counts; len==1 behaves identically, len>1 now counts). Validate additive + hczm pins
  flip. This is byte-neutral for the single-escape corpus.
- **Increment C (unify):** collapse the three collectors into `classify_occurrences` + the §2.3 placement,
  retire the divergent predicates, land the balance assert. This is where it starts SCALING — a new
  boundary/escape shape (wit-abi lift, peer-op result, host-closure call — reference §7) is a new
  `ValueKey` entry point, not a new collector.

Each increment: gated (dev-gate + scoped corpus + CAD for reclaim, reference/placement-doc discipline),
circulated to me for soundness sign-off BEFORE emit, marker/pin flips in the same PR.

## 7. Why this scales (the answer to "the current architecture isn't working")

- **One judgment, not O(shapes) collectors.** Every escape/boundary/capture class routes through the same
  occurrence classification + placement. Closing one closes them uniformly; a new shape is an entry point,
  not a new predicate that can disagree with the others.
- **Multiplicity is first-class.** A count, not a set — multi-escape, multi-consume, and the Nth-consume
  rule are the same arithmetic, no special cases, no `len != 1` punt.
- **The balance invariant is checkable at compile time** — an imbalance is caught by the assert, not
  discovered as a corpus leak or a release OOB weeks later.
- **It is literally Perceus** (svar-dup per occurrence + smatch drop + the borrowed-parameter rule),
  applied uniformly instead of approximated per witness — which is the bar the reference doc sets.
