# Shape inference through `match` unblocks the type-driven emit spine — and prelude variant names must not shadow a program's

*2026-07-07*

**What happened.** Two seed fixes landed together that jointly unblock the compiler's *emit spine* —
the recursive `lower`/`serialize`/`emit` walk that turns a typed AST node into its instruction bytes.

1. **`shape_of` now handles a `match` expression.** Previously a `match` arm that returned a
   freshly-built runtime compound declined with *"cannot infer runtime compound result shape"* — the
   compiler could infer the shape of an `if` (unify the two branches) but not of a `match`. Now a
   `match`'s shape is the *unified* shape of its arm bodies (each arm's pattern binders aliased, exactly
   as `if` unifies its branches; arms that disagree → decline, never a wrong shape). So a
   `match`-arm-returns-fresh-compound infers directly. Verified: a non-recursive `emit` returning
   `(Bytes.of …)` per arm renders `b"B"`; a recursive `emit : Expr → Bytes` that emits a different
   opcode byte per variant and composes sub-emissions with `Bytes.concat` renders correctly
   (`emit (Add (Lit 1) (Neg (Lit 2))) → b"BB|j"`). The `if`-on-discriminant workaround — `match` to
   extract an Int tag, then `if`/build the compound — is no longer needed.

2. **A prelude variant name no longer shadows a program's same-named variant.** `(def (d e) (match e
   ((Expr.Lit n) …) ((Expr.Neg x) (d x))))` over `(type Expr (Lit Int64 | Neg Expr))` was wrongly
   rejected *"a nullary variant carries a non-unit payload"* — because the prelude's `(type Sign (Neg |
   Zero | Pos))` declares a **nullary** `Neg`, and the seed's nullary-variant set was keyed by bare tag
   and add-only, so the prelude's nullary `Neg` shadowed the program's **unary** `Expr.Neg` and misfired
   the arity check. Fixed by making nullary detection last-writer-wins (add when a segment is a lone
   token; remove when a later declaration gives the tag a payload), matching how the payload-kind and
   sum-type maps already override.

**Why.** Both fixes are about the same thing the type-directed-emission work already surfaced in another
guise ([[2026-07-06-result-valtype-is-type-directed-through-an-exhaustive-kind-sum]]): **a compiler
authored as a tree walk needs every property it dispatches on — result shape, result kind, variant
arity — to come from the value's *type/structure*, recovered uniformly wherever the value appears.** The
`match`-shape gap was the last place `if` and `match` diverged in what the compiler could infer through
them, and the emit spine is *built* on `match` (a backend dispatches on the IR node's variant), so the
gap sat directly on the critical path — a recursive `Expr → Bytes` lowering could not be written in its
natural form until a `match` arm returning a fresh `Bytes` inferred its shape. The variant-name
collision is the same lesson from the opposite side: a self-hosted compiler's AST *will* reuse names the
prelude already uses (`Neg`, `Lit`, `App`, `Add`), so any check keyed by a bare, un-namespaced tag will
misjudge the program's type against the prelude's. Last-writer-wins is the interim fix (arity is the
property the check actually needs); proper per-type variant namespacing is the deeper fix, deferred. The
combined effect is that the emit spine — the single most important shape in a self-hosted compiler — now
compiles in its idiomatic form: an exhaustive per-variant `match` that builds each node's bytes and
concatenates sub-node bytes, no workaround.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a recursive emitter dispatches
on a sum's variants to build bytes per node"* — pins the emit spine: a three-variant `Expr` lowered by a
recursive `emit` that returns a distinct freshly-built byte fragment per variant (`Lit → [0x42]`,
`Neg → operand ++ [0x7C]`, `Add → a ++ b ++ [0x6A]`, post-order, matching wasm stack discipline),
`emit (Add (Lit 1) (Neg (Lit 2))) → b"BB|j"`. It is deliberately distinct from the LEB128 encoder cases
(which recurse on an *integer's bits*): this recurses on a *sum's structure*, and each arm builds a
fresh compound whose shape the compiler now infers directly from the unified arm bodies — the exact
capability fix 1 added. It PASSES. The variant-name-collision fix is already pinned by a sibling case
(*"a program's unary variant reusing a prelude nullary variant name is unary"* in `05-compound-types.sexp`).
Together they close SEED-GAPS Tier 3a and remove the last soft blocker on the emit spine; the remaining
gate before the reader is the built-in-Option-across-a-boundary decline
([[2026-07-07-the-built-in-option-loses-its-payload-kind-across-a-boundary]], SPEC-BACKLOG item 12).
