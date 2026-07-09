## 1. 🟡 Pattern binders must compose (nest) — SEED BEHAVIOR NOW LANDED; only the spec MUST remains

**Finding.** `core-semantics.md` §Tuples requires `(tuple a b)` in pattern position binds the
elements, and §Sum Types requires a sum pattern has the form `(Ctor binder)`. Neither says a
*binder may itself be a compound pattern* — i.e. that patterns **nest**: `(Ctor (tuple a b))`,
`(tuple a (tuple b c))`, `(Ctor (tuple op (tuple a b)))`. Yet the corpus already leans on nesting
(`((T.Pair (tuple a b)) …)`), and the compiler's own `resolve`/`lower` front rung is written
`((Node.NPrim (tuple op (tuple a b))) …)` — a tuple nested inside a sum payload. The seed today
binds a *flat* payload tuple but declines a *nested* one (SEED-GAPS Tier 2b), and there is no
in-language workaround (a bare runtime-tuple match arm also declines).

**Why it touches the spec.** Pattern nesting is currently *implied* by the corpus but *not required*
by a MUST. A generation could bind flat patterns, pass every flat-pattern case, and still be unable
to compile the compiler — with no requirement it violated. The staple "a tagged node carrying a tuple
of sub-nodes, destructured in one arm" is the shape every tree-walking pass takes; it should be a
normative requirement, not folklore.

**Proposed resolution (🟡).** Add to `core-semantics.md` §Pattern Matching a requirement that
*patterns compose*: a constructor pattern's binder and a tuple pattern's element MAY themselves be
any pattern (a wildcard, a name, a tuple pattern, or a constructor pattern), matched recursively;
binding is the union of the sub-patterns' bindings (still linear per `CDZ0102`). This is a pure
clarification of intended behavior, not a new feature.

**Status.** Corpus case already added and pinned: *"a match arm binds a nested tuple inside a sum
payload"* in `05-compound-types.sexp` (→ 34, tagged `sum-type-declaration`; scores *todo* — the seed
declines cleanly today, turns green when `bind_sum_payload` recurses). The **spec MUST is the open
item.** Seed fix tracked as SEED-GAPS Tier 2b. Learning:
`spec/learnings/2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md`.

**Update (2026-07-06, late):** this is now demonstrably the *only* thing between a working backend and
a compiler driven from a real surface tree. The spike proved the backend by ROUTING AROUND it — `main`
hand-builds the folded `Core`/`Func` list `resolve` would produce and drives the multi-function
assembler, emitting a valid 2-function component (`main = dbl(21)` → 42 via a real `call`). So the
downstream (assemble → frame, calls, params, runtime conditionals) is proven from the resolved IR
inward; only the surface→`Core` front rung stays stubbed behind this nested-binder gap. Fixing it (make
`bind_sum_payload` recurse into a nested `(tuple …)` binder) unblocks the front end end-to-end. Learning:
`spec/learnings/2026-07-06-the-compiler-emits-a-multi-function-module-with-a-real-call.md`.

**Update (2026-07-07) — SEED FIX LANDED; downgraded 🔴→🟡.** `bind_sum_payload` now recurses into a
nested `(tuple …)` binder (via `bind_tuple_elems` + a new `sum_payload_types` map carrying per-slot type
nodes), so a nested payload binds to any depth. The corpus case *"a match arm binds a nested tuple inside
a sum payload"* flipped **todo → PASS** with no edit to the oracle (reject-don't-miscompile working as
designed). The spike's front end is now **closed end-to-end**: a `Def`/`DList` multi-def surface +
`resolve-module` compiles `(module m (def (main) (+ 20 22)) (def (dbl x) (* x 2)))` to a valid 2-function
component. **What remains for the operator is only the spec MUST** — whether `core-semantics.md` §Pattern
Matching should carry an explicit requirement that patterns compose (a constructor/tuple binder may
itself be any pattern, matched recursively), now that the behavior exists and is gate-pinned. Purely a
"make the requirement explicit" call, no behavior change. Learnings:
`spec/learnings/2026-07-07-the-nested-payload-binder-fix-closes-the-front-end.md` (and the seed-side fix
in memory `nested-tuple-binder-in-sum-payload`).

---

---

## ✅ DONE 2026-07-07 (conformance loop) — the spec MUST landed (seed behavior was already in)

Added the normative **"Patterns Compose"** requirement to `core-semantics.md` §Pattern Matching (name-free,
three MUSTs): (1) a pattern MUST admit any pattern in each binder position (constructor binder / tuple element
MAY be wildcard/name/tuple/constructor), matched recursively to any depth; (2) a composed pattern MUST bind the
UNION of its sub-patterns' bindings and remain LINEAR (a name in two sub-patterns = the same `CDZ0102`);
(3) destructuring a tagged value carrying a tuple of sub-values in one arm MUST be expressible as one nested
pattern, not a bind-then-rematch.

**Verified:** the seed already realizes it — `(match p ((P.Pair (tuple a b)) …))` and the deeper
`((N.Prim (tuple op (tuple a b))) …)` both compile VALID; the corpus case "a match arm binds a nested tuple
inside a sum payload" now PASSES (gate 573/0). Spec-only change → seed binary/ignition/component-check/cargo
test unchanged from last cycle's green. Spec name guard clean. Learning: `patterns-compose-spec-must`. This
closes ask-01 (the seed side was the implementation learning
`2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md`, done; this is the spec
MUST that was the sole remaining open item).
