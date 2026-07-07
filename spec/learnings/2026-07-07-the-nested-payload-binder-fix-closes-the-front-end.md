# The nested-payload-binder fix closes the front end — a multi-def surface module now compiles end-to-end

*2026-07-07*

**What happened.** The nested-tuple-binder blocker (Tier 2b — a `match` arm destructuring a sum
payload whose binder is itself a tuple, `(Ctor (tuple op (tuple a b)))`) is **fixed in the seed**.
`bind_sum_payload` now recurses into a nested `(tuple …)` slot — it reads the slot's heap handle and
destructures it by the same slot logic — exactly the fix predicted when the gap was first isolated
([[2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders]]). The corpus case
that pinned it (*"a match arm binds a nested tuple inside a sum payload"* in `05-compound-types.sexp`, a
recursive `Expr` evaluator over `Bin (Tuple Int64 (Tuple Expr Expr))`) flipped from *todo* to **PASS**
with no change to the case — the recorded oracle (→ 34) was right all along; only the seed needed to
catch up. This is the reject-don't-miscompile discipline working as designed: the gap was recorded as a
declining case with its true output, and the fix turned it green.

With the binder fixed, the compiler-in-Cadenza spike **closed its front end end-to-end**. It added a
multi-definition surface layer — a `Def` (`name`, `param-count`, `body-Node`) and a `DList` of them,
exactly what a reader produces — and `resolve-module` walks that `DList` to the typed `FList` the
multi-function backend consumes, resolving each def's body `Node → Core` and each head *name* to a code.
A whole textual multi-def module now flows the complete pipeline: **read → resolve → fold → lower →
serialize → frame → component bytes.** Verified: `(module m (def (main) (+ 20 22)) (def (dbl x) (* x 2)))`
compiles to a valid 2-function component — func 0's `(+ 20 22)` folds to `i64.const 42`, func 1's
`(* x 2)` stays a runtime `local.get 0; i64.const 2; i64.mul`, every head resolved from its name string.
The only remaining piece before self-hosting is the **reader** (input bytes → `DList`, i.e. CBOR decode
of the canonical AST).

**Why.** Tier 2b was the last *structural* blocker on the front rung, and its fix is what let the front
end close: a compiler's resolve/lower passes are fundamentally "a tagged node carrying a tuple of
sub-nodes, destructured in one arm and walked recursively," so a compiler cannot decode its own AST
until a nested payload binder both compiles and binds correctly. Two design notes worth preserving.
First, the spike still keeps its surface nodes **flat** where it can — `Def` is a flat 3-tuple
`(name, np, body)` and `NPrim` is `(String, Node, Node)` — not because nesting still declines (it no
longer does) but because a flat payload is the simpler shape and the reader produces head-plus-operands
naturally; the nested binder is now *available* for the genuinely-nested cases (a sum payload that is a
tuple of sub-nodes) rather than *required* everywhere. The stale "nesting declines" comments in
`compiler.cdz` are now false and should be pruned. Second, the front end's closure is real but its error
channel is still a stub: an unrecognized head resolves to `PUnknown`, and `resolve` "declines" on it by
constructing an out-of-range `Bytes` value to force a runtime trap (`unknown-head-trap`) — a placeholder
standing in for a proper compile-time diagnostic (a `CDZ` code) the front end does not yet have. That is
honest as an interim (it does halt rather than miscompile), but it is a runtime trap where a *compile-time
rejection* belongs, so it is a backlog item, not a finished behavior.

**The requirement it drove.** No new corpus case — the Tier 2b fix is witnessed by the existing
recursive nested-binder case now passing, which is the stronger outcome (a recorded decline turned green
without editing the oracle). The durable record is this learning plus two backlog updates in the spike's
`SPEC-BACKLOG.md`: **item 1 (pattern-binder nesting) moves toward resolved** — the seed now binds nested
payloads, so the remaining question is only whether `core-semantics.md` should carry an explicit
*requirement* that patterns compose (the MUST), now that the behavior exists and is gate-pinned; and a
**new item** records that the front end's unknown-head path needs a real diagnostic rather than the
`unknown-head-trap` placeholder. The milestone itself — front end closed end-to-end, only the reader
remains — is the headline: the compiler-in-Cadenza now spans surface text to component bytes for the
arithmetic/comparison/conditional/multi-function subset, and self-hosting is gated on the CBOR reader and
the remaining runtime-value work (float equality, compound result shape), not on any front-rung
structural gap.
