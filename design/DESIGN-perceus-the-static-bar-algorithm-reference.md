# Perceus: the static reclaim discipline — the bar rcdzc must meet

Status: REFERENCE (v-memory-safety, 2026-08-29). This is the **ground-truth** specification of the
reclaim algorithm every reclaim change in this compiler must conform to. It is distilled directly
from *Perceus: Garbage Free Reference Counting with Reuse* (Reinking, Xie, de Moura, Leijen; MSR
Technical Report MSR-TR-2020-42, 2020-11-22, update v4 2021-06-07) — the same algorithm Koka and the
Lean prover use. When a reclaim placement is proposed, cite the rule here it realizes; if it does not
correspond to a rule here, it is not Perceus and it is very likely a leak or a use-after-free.

Read this **before** editing `backend/wasm/select.rs`'s dup/drop/escape/reuse machinery, and before
proposing a new "gate" for a leak/UAF class. The companion docs
[`DESIGN-perceus-loop-match-reclaim-placement.md`](DESIGN-perceus-loop-match-reclaim-placement.md) and
[`DESIGN-loop-and-sum-heap-reclaim-alias-analysis.md`](DESIGN-loop-and-sum-heap-reclaim-alias-analysis.md)
are the *placement* half — where drops land for specific leak families. THIS doc is the *why*: the
uniform owned/borrowed judgment those placements are all approximating. The recurring leak/UAF classes
exist because the placements are a patchwork of narrow exceptions around a wrong default, not a uniform
application of the rules below.

Normative language (MUST/MUST NOT/SHOULD) is used deliberately. The invariants in §1 and §6 are hard.

---

## 0. The one-line summary

**Reclaim is FULLY STATIC.** `dup` (increment) and `drop` (decrement/free) are inserted at
**compile time** by a syntax-directed pass. **A `dup`/`drop` decision MUST NEVER branch on a runtime
reference-count value.** (Runtime *does* branch inside a single `drop` — "is this the last reference?"
— but *whether the compiler emits a drop at all*, and *how many dups*, is decided statically from the
program's structure. The two are different; conflating them is the error that produced the retracted
snowflake "rc-conditional dup".) A missing `drop` or an extra `dup` is a **leak**; a missing `dup` or
an extra `drop` is a **use-after-free**. Both are static insertion bugs, fixable only by getting the
static owned/borrowed judgment right — never by a runtime rc test in the emitter's decision.

---

## 1. The core idea (paper §2.2): ownership is passed IN

- Every heap value has a reference count; it is freed **the instant** its count hits 0 (this is what
  "precise" / "garbage free" means: at every non-rc step, `rc(x)` = the exact number of live references
  to `x`, and every heap value is reachable — Thm 2 and Thm 4).
- **Ownership transfers into functions.** A parameter is **owned** by default: the callee is
  responsible for freeing it. This is why `map(xs, f)` frees `xs` itself and the caller emits no drop
  — `xs` is consumed by `map`. Contrast C++ `shared_ptr`/Rust `Rc`/Nim, which tie a value's lifetime to
  its lexical *scope* (drop at end of block); that retains memory longer than needed. Perceus frees
  eagerly: in `map`, `xs` is deallocated *node by node as the new list `ys` is built*, halving peak
  memory.
- Consequence, and the crux of most of our bugs: **an owned value is consumed EXACTLY ONCE.** If it is
  needed twice, it must be `dup`'d. If it is not needed, it must be `drop`'d. If it is passed on, it is
  moved (no rc op).

The Cadenza value heap is **acyclic** (immutable values, recursion via static code references — see
`spec/capabilities/memory-and-resource-model.md`), so reference counting is *complete*: it reclaims
everything, no cycle collector needed. Perceus is therefore not "one option among GC strategies" for us;
it is the whole reclaim story, and it must be correct.

---

## 2. The algorithm — syntax-directed rules (paper Fig 8)

The judgment is `Δ | Γ ⊢s e ⇝ e′`: under **borrowed** environment `Δ` and **owned** environment `Γ`,
expression `e` is rewritten to `e′` with explicit `dup`/`drop` inserted. Invariants maintained *by
construction* at every step (paper §3.4):

1. `Δ ∩ Γ = ∅` — a variable is never simultaneously borrowed and owned in the same judgment.
2. `Γ ⊆ fv(e)` — you only own what the expression actually uses.
3. `fv(e) ⊆ Δ, Γ` — every free variable is accounted for as borrowed or owned.
4. Multiplicity of every member of `Δ, Γ` is 1.

The two guiding heuristics (both matter for correctness AND for the optimizations in §4): **dup as LATE
as possible** (push dups toward the leaves / to just before the consuming use) and **drop as SOON as
possible** (right after a binding becomes dead, or at the START of a match branch).

The rules (owned var on the right of the turnstile is the resource being placed):

- **svar** — `Δ | x ⊢s x ⇝ x`. `x` is owned and used exactly once: consume it, emit no rc op. (The
  fast path: a value flows straight through with zero overhead.)

- **svar-dup** — `Δ, x | ∅ ⊢s x ⇝ dup x; x`. `x` is **borrowed** (in `Δ`) but the position needs an
  owned value → **emit `dup x`**. *This is THE borrowed rule.* Borrowing a value into an owned use is a
  dup. Almost every one of our escape/boundary UAFs is a missing application of svar-dup.

- **sapp** — for `e1 e2`: `Γ2 = Γ ∩ fv(e2)`; derive `e1` under `(Δ, Γ2 | Γ − Γ2)` (so the vars still
  live in `e2` are *borrowed* while deriving `e1`), derive `e2` under `(Δ | Γ2)` (owned). Deterministic
  split. This is what lets the dup be pushed as late as possible: `λ f g x. (f x)(g x)` becomes
  `λ f g x. (f (dup x; x)) (g x)` — the dup lands right before the first consuming use of `x`, not up
  front.

- **slam** / **slam-d** — for `λx. e` with `ys = fv(λx.e)`, `Δ1 = ys − Γ`: a lambda is a closure
  holding its free variables, so the free vars NOT already owned (`Δ1`, the borrowed ones) MUST be
  `dup`'d to move them into the closure: `dup Δ1; λ ys x. e′`. **slam-d** handles an unused parameter:
  `λ ys x. (drop x; e′)` — drop the unused param at entry.

- **sbind** / **sbind-d** — `val x = e1; e2`. If `x ∉ fv(e2)` (unused binding): **sbind-d** emits
  `val x = e1; drop x; e2′` — drop it immediately. Otherwise **sbind** threads it like sapp.

- **smatch** (THE key rule) —
  `Δ | Γ, x ⊢s match x { pi → ei } ⇝ match x { pi → drop Γi′; ei′ }`
  where `Γi = (Γ, bv(pi)) ∩ fv(ei)` (the owned vars, INCLUDING the pattern-bound fields, that are LIVE
  in branch `i`) and `Γi′ = (Γ, bv(pi)) − Γi` (the owned vars DEAD in branch `i`). So **each branch
  drops its dead owned variables at the branch START** — this includes the scrutinee `x` once its
  fields are bound, and any pattern binding the branch does not use. Bound fields that ARE used stay
  owned and flow on.

- **scon** — `C v1 … vn`: split `Γ` per component; each component borrows the environments of the
  components derived after it (dup as late as possible, per sapp).

### The heap-semantics form of match (paper Fig 7, rule `matchr`) — the operational truth

`H | match x { pi → ei } −→ H | dup ys; drop x; ei[xs := ys]` where `(x ↦→n C ys) ∈ H`.

In words: on matching `x = C(ys)`, **`dup` the bound fields you are keeping, then `drop` the
scrutinee**. The scrutinee drop, when it reaches rc 0, **cascades** (`dconr`: `drop x; e −→ drop ys; e`
— free the cell, then recursively drop its fields). The field-dups you emitted balance the cascade for
the fields you kept; the fields you did *not* keep are freed by the cascade. This is the single most
important operational identity for us:

> **dup-the-kept-fields + drop-the-scrutinee is a balanced, complete reclaim of a matched value.**
> A drop of the shell WITHOUT the kept-field dup is a use-after-free (the cascade frees a field you
> still hold). A kept-field dup WITHOUT the shell drop is a leak (the shell is never freed).
> These two ALWAYS go together. (This exact coupling is why the placement doc's site-A / non-tail-spine
> fixes insist "dup ⟺ shell-drop are INSEPARABLE".)

Also from Fig 7: `appr` duplicates the closure's captured env `ys` and drops the closure `f` on call;
`dlamr`/`dconr` are the cascade rules; values are allocated at rc 1 (`lamr`/`conr`).

---

## 3. The borrowed-parameter rule (paper §3.2 "let!", and the `sapp`/`sbind` borrow)

There are exactly two static parameter modes, and the whole boundary leak/UAF family is about applying
them consistently:

- **OWNED parameter (the default).** The callee drops it. A `match` of an owned param is `smatch`
  above: dup the kept fields, drop the scrutinee, cascade.

- **BORROWED parameter (the `let!` / borrow optimization).** The callee does **NOT** drop it — the
  **owner (the caller / the enclosing binding) drops it**. This is Wadler's `let!`: a linear value can
  be *borrowed* as a regular value for the duration of a sub-derivation, because we know it is still
  alive afterwards for the owner to reclaim. BUT: **any field extracted from a borrowed value that is
  then used as owned — consumed, returned, or stored into a constructor — MUST be `dup`'d** (that is
  exactly `svar-dup` applied to the extracted field). Borrowing is what lets us delay the dup; it does
  not remove it.

⇒ **A borrowed scrutinee whose bound field escapes needs BOTH:** (1) `dup` the escaping field
(svar-dup), AND (2) the owner/caller drops the scrutinee. Both are static, keyed on the field
escaping — **never on a runtime rc read.** This is the correct, uniform framing of the snowflake UAF
(see `snowflake-*` memory + [`DESIGN-loop-and-sum-heap-reclaim-alias-analysis.md`]): the escaping
sum-payload of a boundary-owned scrutinee was extracted without the svar-dup, so when the owner's drop
cascaded it freed a field still live in the returned value.

---

## 4. The optimizations (all STATIC, all sound by erasure `⌈e′⌉ = e`)

These do not change WHAT is reclaimed; they remove rc *operations* on the fast path. They are the
reason precise rc is competitive with tracing GC (paper §4).

- **Drop specialization (§2.3).** Inline `drop x` specialized per constructor:
  `drop x = if is-unique(x) then { drop children; free } else decref`. Then push dups down into branches
  and **fuse** matching `dup`/`drop` pairs. On the unique fast path (rc == 1, the common case for a
  freshly built or linearly-threaded value) the node is freed with *zero* further rc ops. NOTE: the
  `is-unique` test here is a runtime test INSIDE one drop's implementation — it is NOT the emitter
  deciding whether to place a drop. §0's ban is on the latter.

- **Reuse analysis / FBIP (§2.4).** In a match branch, pair the matched constructor with a same-size
  constructor allocated in the branch. If the scrutinee is not live, emit `drop-reuse(x)` → a **reuse
  token**; the branch's constructor is allocated `Ctor@token`, which at runtime reuses `x`'s cell
  in-place if `x` was unique (`token != NULL`) and mallocs otherwise. This is how purely functional code
  (map, tree rebalance, list reverse) runs **in place** when the input is unique, and copies exactly the
  shared spine when it is shared. "Functional But In Place" (FBIP).

- **Reuse specialization (§2.5).** When the reused constructor keeps most fields the same
  (`Node(Red, ins(l,k,v), kx, vx, r)` reuses `t` and only re-assigns `left`), specialize so only the
  changed field is written. Only specialize if ≥1 field stays the same.

**FBIP is invisible to a naive escape analysis and is a real UAF source in rcdzc** (placement doc §"FBIP
reuse is invisible"): a single-use consuming constructor silently takes its operand's cell into the
result via the fast path; a subsequent shell deep-drop then frees a cell aliased into the live result.
Any shell-reclaim decision MUST be **reuse-clean** (no FBIP reuse in the arm takes a cell reachable from
the shell being dropped) as well as escape-clean.

---

## 5. Non-linear control flow (paper §2.7.1) — why explicit control flow is required

Perceus needs **explicit** control flow to know statically where dup/drop go. Exceptions / non-resumed
continuations break this: if `f(x)` throws and exits `map`'s scope, `xx` and `f` would leak. Koka's fix
(and ours): compile all control effects to explicit control flow *before* the reclaim pass, so every
path that can exit is a visible branch with its own drops. In Cadenza this is the effect/handler
lowering: `resume` is spliced away in `reduce_handle`/`splice_context` **before Core**, so at select.rs
there is no resume node. That is exactly why the effects resume-escape leaks (14c) are hard — the
"old state is replaced" / "value escapes forward into the continuation" signal is Core-invisible, so a
conservative HOLD (leak, not reclaim) is the only sound choice there until the signal is exposed at the
lowering seam (v-effects' lane; see §2c.3 of the placement doc). This is a genuine "correct to leak"
class, not a fixable gap — do not force a drop there.

---

## 6. The bar, stated as hard invariants (what any reclaim change MUST satisfy)

1. **Static only.** The emitter's decision to place a `dup`/`drop`, and how many, MUST be a function of
   program STRUCTURE (owned/borrowed judgment, escape, FBIP reuse) — NEVER of a runtime rc value.
   (Runtime `is-unique` *inside* a single drop/reuse-token is fine; that is §4, not a placement
   decision.)
2. **Balanced.** Every `dup` has a matching eventual `drop` and vice-versa, on every control path. The
   matchr identity (§2) — dup-kept-fields ⇔ drop-scrutinee — is the canonical balanced unit; do not
   break half of it.
3. **Owned-vs-borrowed is uniform.** A reclaim change must correspond to one of: svar-dup (borrow→own
   dup), smatch (branch-start drop of dead owned incl. scrutinee), sbind-d/slam-d (drop unused binding
   at once), the borrowed-parameter rule (§3: dup escaping field + owner drops), or an §4 optimization.
   If it corresponds to none of these, it is not Perceus.
4. **Leak beats UAF (correctness bias).** A wrong "safe to reclaim" is a double-free (a trap / silent
   corruption); a wrong "unsafe" is a leak (value-correct). When the static judgment cannot PROVE safety,
   HOLD (leak). Widen only when the proof exists.
5. **Immutable constants are build-once-immortal, not reclaimed.** A nullary variant of a mixed sum, an
   empty vec, `unit` — constant data with no varying payload — is marked immortal (rc = sentinel,
   census-excluded, dup/drop no-op) rather than reclaimed per-construction. This is sound because rc ≠ 1
   ⇒ FBIP never mutates it (it path-copies). See the immortal-nullary-terminal landing (#4785) and the
   immortal-empty-vec work. Immortalizing is the RIGHT representation for a constant terminal, and it is
   robust to whichever rc the node ends up at (moots the "over-ref vs under-drop" debate for that class).

---

## 7. Gap map — where rcdzc DEVIATES from the rules, by class

rcdzc does **not** implement the uniform owned/borrowed judgment of §2–3. Instead
`heap_operand_ownership(Param)` **defaults to BORROWED** (`select.rs`, ~L17542) — the *inverse* of
Perceus's owned default — and then adds narrow OWNED exceptions and piecemeal escape-dups. The gaps
*between* those exceptions are the leak/UAF classes. Each recurring class maps to a missing rule:

| Class (witnesses) | Missing Perceus rule | Correct fix |
|---|---|---|
| Boundary / lifted escape UAF (snowflake) | **svar-dup** on the borrowed scrutinee's escaping field + **owner drop** (§3) | dup the escaping sum-payload at extraction; owner drops the boundary arg. Static, keyed on escape — NOT a runtime rc test. |
| Recursive-fold terminal shell (depth-tail, ss1, rsl1) | constant terminal should be **immortal** (§6.5), not reclaimed | build-once-immortal the nullary/empty terminal (landed #4785 for SumNew; empty-vec analog for built-in List). |
| Self-loop-tail spine (~333) | **smatch** drop-scrutinee on the back-edge + last-use-no-dup (over-dup today) | consume-last ordering + skip the preservation dup when the split-consume is the sole last use (placement doc §5, PR #4139). |
| Non-tail recursive-sum spine (sum-nat) | **smatch** on an owned param scrutinee: dup kept payload + drop shell (§2 matchr) | narrow proven-owned-dead-after param-reclaim gate (the non-tail analog), NOT flipping the L17542 default. |
| Fallible-extraction `Some` shell (lar1/mlr1/…) | shell drop needs the **kept-field dup** to balance the extraction's retain (§2 matchr) | release the extraction retain on `Some` unwrap; reuse-clean guards FBIP. |
| Host-closure boundary (21, ~191) | **owner drop** of an `own<t>` handle consumed at the boundary, gated by capture-escape (§3 + §5) | drop the closure rep after `call_indirect`, gated own-not-borrow AND no captured value escaped via the body return (else double-release, hcz1/hcz2). |
| Effects resume-escape (14c, ~17.5K cells) | **§5**: control flow spliced away pre-Core → signal invisible | mostly a handle-exit-drop at the reduce_handle seam gated on unconsumed; the resume-continuation-escape MINORITY is a correct HOLD (leak beats UAF). |
| dqe dual-use over-dup / adv54b missed-2nd-consume | binding-GLOBAL consume-count (Nth-consume dup rule) vs LOCAL per-occurrence marking | one binding-global consume/borrow classification driving dup (one per consuming use after the first with a later live use) + drop (one per borrow-only survivor at end-of-scope). Placement doc §4. |

**The systemic fix the operator is asking for:** replace the BORROWED-default + narrow-exception
patchwork with a **uniform owned/borrowed judgment** (§2–3) computed binding-globally, then place
dup/drop by the rules — svar-dup for borrow→own, smatch for match, borrowed-parameter for the boundary.
Each current point-fix is that rule applied to one shape; doing it uniformly closes the *class*, not the
witness. Until that lands, every point-fix MUST cite the rule above it realizes and MUST hold the
leak-beats-UAF bias (§6.4).

---

## 8. Verification tools the object-census does NOT give you (use these on every reclaim change)

The static `(live-objects N)` census counts OBJECTS, not REFERENCES, so it hides two failure modes:

- **Value-wrong grep.** A miscompile reports `expected (: X), got Y` with NO `trapped:` line — it hides
  among leak-count mismatches. Grep every FAIL that is NOT a `live-objects mismatch`.
- **Flap detection.** Run the corpus TWICE; a live-objects count that DIFFERS run-to-run is a
  census-hidden rc UNSOUNDNESS (order/allocation-dependent imbalance) even when the value is correct.
- **Release-trap ≠ debug-leak.** The debug `--report-live-objects` runtime does NOT trap the
  rc-underflow / over-free class — it shows it as a LEAK. That class only OOBs on the RELEASE runtime
  (CAD / wasmtime). So for an over-free/UAF hunt, trust the CAD/release trap, not the debug leak count
  (see `debug-runtime-does-not-trap-on-rc-underflow-over-free-class-*` memory).
- **Witnesses live in the CORPUS** as `(live-objects N)` cases, not rcdzc rust `#[test]`s — rcdzc has no
  wasmtime dep and does not execute wasm. Behavioral reclaim verification is corpus-by-design.

---

## 9. Where the code is (as of 2026-08-29, verify line numbers — they drift)

- `backend/wasm/select.rs` — all dup/drop/escape/reuse/loop-drop machinery (single-writer: v-memory-safety
  directs; v-core-opt executes the emit).
  - `heap_operand_ownership` (~L17542) — the BORROWED-default that inverts Perceus (§7).
  - `binding_escapes` / `_dup_aware` (~L1280/1308) — the escape analysis (borrow-vs-consume threading).
  - `arm_borrows_heap_subvalue` (~L15861/17097) — syntactic heap-subvalue read in consume/result position.
  - `mark_binder_dups` / `collect_dup_sites` (~L1935/2075) — per-occurrence dup marking (the LOCAL
    approximation §4/§7 want made binding-global).
  - `sum_shell_reclaim_ok`, `collect_shell_reclaim_child_dups`, `list_shell_reclaim_slot`,
    `looped_owned_param_drops`, `param_only_borrowed_or_backedge` — the narrow OWNED exceptions.
- `cdz-runtime/src/lib.rs` — the runtime rc ops (`op_dup`/`op_drop`/`op_sum_new`/`op_vec_*`), the
  `IMMORTAL` sentinel + census (`op_mark_immortal` decrements `LIVE_NODES`), the rc==1 FBIP reuse gate.
  Frozen-hash: editing `//` comments / `wit/runtime.wit` bumps `REQUIRED_RUNTIME_HASH`.
- The `#4635` UAF detector (expanded to a live-node assert at every getter) is the fleet-wide backstop —
  an over-drop/read-through-freed traps at the exact getter on the debug-counters store.

---

## 10. TL;DR for a reviewer / a teammate touching reclaim

1. Is the decision **static** (structure, not runtime rc)? If it branches on `rc(x)` to decide whether to
   emit a dup/drop → REJECT (§0/§6.1).
2. Which **rule** (§2–3) does it realize? If none → it is not Perceus → suspect a leak/UAF.
3. Is it **balanced** — every dup matched by a drop on every path; matchr's dup-kept ⇔ drop-scrutinee
   kept intact (§6.2)?
4. Is it **escape-clean AND reuse-clean** (§4) — no FBIP alias into a dropped shell?
5. When the proof is missing, does it **HOLD (leak)** rather than reclaim (§6.4)?
6. Verified with **value-wrong grep + flap-detect + release/CAD trap**, not just the object census (§8)?

If a teammate needs this context, point them here. The bar does not move.
