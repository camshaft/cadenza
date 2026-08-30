# Perceus OccTable — the effects RESUME-SEAM occurrences (v-effects slice)

Status: DESIGN (v-effects, 2026-08-30). The resume-seam slice of the uniform per-occurrence
reclaim architecture — read
[`DESIGN-perceus-per-occurrence-dup-placement-uniform.md`](DESIGN-perceus-per-occurrence-dup-placement-uniform.md)
first; that doc lists v-effects as CO on the boundary / **resume seams** (its §0 line "v-effects/
v-rust-backend (the boundary/resume seams)"). This enumerates the heap values the effect fold's
resume-lowering produces, their occurrences, the `ValueKey` each maps to, and the borrow/consume
class at each — so v-core-opt's `select.rs` emit sees a classified `OccTable` slice for the Core it
cannot introspect back into the fold.

Owner (soundness/acceptance): v-memory-safety. Executor (select.rs emit + placement): v-core-opt.
This slice (resume-seam classification + witnesses): v-effects. Slots into #5857 **Increment C**
(the unify step) — Increment A (hczm capture-escape UAF) and B (dup_at representation) are
v-core-opt's and land first.

---

## 0. Why the resume seam needs its own enumeration

The reclaim's occurrence classifier walks Core. But the effect fold REWRITES a `handle`/`resume`
into ordinary Core (multi-value returns, `#st` tuples, threaded state params, and — when a
continuation escapes — a synthesized closure) BEFORE `select.rs` runs. So at emit time the
"resume" is invisible: what `select.rs` sees is a `(tuple value state…)` build, tuple projections,
a fn-param thread, or a closure capture. The heap values that cross the resume boundary must
therefore be classified WHERE the fold mints them (effects.rs), not rediscovered at select.rs. This
doc names those values so they become `OccTable` entry points, not a new escape predicate.

## 1. The resume-seam heap values (what the fold mints)

Grounded in the fold's resume-lowering (effects.rs):

1. **The `#st` value/state tuple.** `build_value_state_tuple` (effects.rs:5588) packs
   `(tuple value out_state…)`; `peel_tuple_value_state` (10500) rewrites `(resume v s)` →
   `(tuple v s)`. On each dispatch the tuple is built, returned multi-value, and unpacked
   (`tuple_proj`) into the next step's `value` and `state`. HEAP-relevant when `value` or any
   `state` is heap-typed (list/tuple/map/set/sum — e.g. `su6d`/`crn1`'s tuple state, a rope
   String state).
2. **The threaded state binder(s).** The handler-arm state `s` (and, in a merged nested context,
   one slot per handler — `merged_nested_ctx`, effects.rs:600) is threaded as a fn PARAMETER through
   the specialized dispatch loop (see the state-parameter threading, §"a specialized fn threads it
   through", effects.rs:512). Each dispatch READS it (to compute the next state) and PRODUCES the
   next one (packed into the next `#st`).
3. **The escaping-continuation closure.** When a continuation escapes (not tail-resumptive), the
   fold synthesizes `(fn (#kv) C)` where `C = body[perform := #kv]` (effects.rs:2686–2700) — a
   closure whose ENV captures the threaded state + any arm binders live across the resume. This is
   the DES stored-`k` / deferred-resume shape.
4. **The resume `value`.** The `v` in `(resume v s)` — the value handed to the continuation's next
   step; consumed there.

## 2. ValueKey mapping (reuse the OccTable enum, no new kind)

Using the reference doc's `enum ValueKey { Param | Let | Captured | PayloadNode }`:

| resume-seam value | ValueKey | rationale |
|---|---|---|
| threaded state binder `s` (fn-param thread) | `Param(s)` | it IS a synthesized fn parameter of the dispatch loop |
| `#st` unpack `value`/`state` (per dispatch) | `PayloadNode(proj)` | a projection out of the `(tuple …)` per dispatch — the same shape as a sum-payload extraction the reference doc already models |
| continuation closure capture (state + arm binders live across resume) | `Captured(i)` | it is a closure-env slot, identical to any other capture |
| resume `value` / next-state build operands | (the enclosing `PayloadNode`/`Param` of the operand) | not a new key — an occurrence of an existing value |

So NO new `ValueKey` variant — the resume seam is three existing entry points (param-keyed for the
threaded state, payload-node-keyed for the `#st` unpack, capture-keyed for the escaping closure)
seeding the same classifier.

## 3. Borrow vs Consume AT the resume seam (the classification)

Reusing the reference doc's per-occurrence arm logic (Borrow = read-without-retain, Consume =
ownership out); the resume-specific judgments:

- **`resume v s` CONSUMES both `v` and `s`** — they are moved into the next `#st`/dispatch (an
  ownership out-flow), so each is a `Consume`. If the same `s`/`v` is ALSO read earlier in the arm
  (e.g. `(resume (. s 0) (tuple (. s 0) …))` in `crn1`), those reads are `Borrow`; only the
  threaded-into-next occurrence is the Consume.
- **A pre-resume state READ is a Borrow** — `(. s 0)`, `(match s …)`, `s.len` inspect without
  retaining (the state is threaded onward intact).
- **The escaping-continuation closure capture is BORROWED-init (`init = 0`)** — the closure ENV
  owns the captured state and drops it on the closure's own drop, exactly like any closure-env-owned
  capture in the reference doc §2.3. So EACH escaping occurrence of the captured state gets `+1 dup`
  (the borrowed-value rule) — not the owned last-move rule.

## 4. glb1 IS the §2.3 borrowed-N-escapes rule (the acceptance case)

glb1 (the collapse-continuation duplication join-point I have tracked as reclaim-coupled and parked)
is a continuation reachable via a JOIN POINT that duplicates its captured `#st` state across N
control-flow paths. Under this enumeration:

- the captured `#st` state is a `Captured` value with `init = 0` (borrowed — the closure env owns it);
- the join duplicates it into N escaping occurrences;
- §2.3 borrowed rule ⇒ **N dups, one per escaping occurrence, PATH-AWARE** (mutually-exclusive
  collapse arms take the MAX, not the sum — the reference doc's `ifcap` anti-over-dup note applies
  verbatim: a continuation reached via both arms of the collapse needs ONE dup on the taken path).

So the OccTable SUBSUMES glb1 — no separate glb1-emit lowering is needed; it becomes a resume-seam
acceptance case, and the parked glb1 pairing folds away. This is the concrete proof that the
resume seam is "just values with occurrences", not a special mechanism.

## 5. Soundness question for v-memory-safety (the one judgment I need signed off)

The whole slice hinges on ONE ownership call: **is the threaded `#st` state, at a resume site,
borrowed (`init = 0`) or owned (`init = 1`) in the dispatch frame?** My read: the continuation
closure env (case 3) and the multi-value return convention mean the threaded state is BORROWED by
the dispatch frame (the loop/closure holds the real ref and drops it), so every resume out-flow
needs its own dup — the borrowed rule, matching glb1. But the tail-resumptive fast path (no escaping
closure; the state is threaded as a plain fn param through a loop) MAY be owned (`init = 1`, the last
dispatch moves it, earlier ones dup) — this is the `heap_operand_ownership(Param)` default the
reference doc §1 flags as BORROWED-by-default. I will propose per-shape `init`:
- tail-resumptive loop-threaded state → `init` follows the existing param ownership (conservative:
  borrowed → dup each, "leak beats UAF" §5);
- escaping-continuation captured state → `init = 0` (borrowed, closure-env-owned) always.

v-memory-safety: please confirm/correct the `init` for the tail-resumptive threaded-state param —
that is the single soundness pin the rest derives from.

## 6. Acceptance witnesses (the resume-seam corpus for Increment C)

- `rsh1` (reduce-share, PR #5829) + the same-effect straddling twins (heap-state PR #5883,
  compound-resume `crn1` PR #5885) — nested-handler state + compound-resume threading, all probed
  sound (v-effects a45/a46): these must stay PASS-value + `live-objects 0` under the OccTable.
- The `#5090` flagship-reducer UAF fences + SITE-A over-retention (`#5142`) — the resume-seam UAF
  witnesses; must stay 0.
- `glb1` — flips from parked-decline/leak to PASS `live-objects 0` under the borrowed-N-escapes rule
  (the acceptance proof for this slice).
- Path-aware control: a continuation escaping via BOTH collapse arms → ONE dup (the resume-seam
  analogue of `ifcap1`) — proves no over-dup leak.

## 7. Next steps

1. v-memory-safety signs off §5 (the `init` judgment for the tail-resumptive threaded state).
2. v-effects implements `classify_occurrences`' resume-seam entry points (the three §2 seeds) +
   the `glb1` acceptance case, feeding v-core-opt a classified slice — NOT touching `select.rs`
   emit or the placement rules.
3. Lands as part of #5857 Increment C, behind v-core-opt's Increment A (hczm) + B (dup_at repr).
