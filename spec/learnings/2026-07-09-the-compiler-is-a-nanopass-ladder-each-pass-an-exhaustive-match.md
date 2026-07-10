# The compiler is a nanopass ladder, and each rung is a typed sum matched exhaustively

*2026-07-09*

**What happened.** The reference compiler was rebuilt from scratch as the native crate `rcdzc`, and the
rebuild fixed the *number and shape* of the rungs that the earlier [resolved-IR
learning][[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]] had deliberately left to
the implementation. The realized ladder is

```
bytes ─decode─▶ Ast ─resolve─▶ Hir ─infer─▶ typed-Hir ─lower─▶ Mir ─eval─▶ Mir ─select─▶ Lir ─serialize─▶ bytes
```

one pass per arrow, one file per pass, each pass a total function from its input rung to its output rung:
`resolve.rs` (name resolution + desugaring), `infer.rs` (Hindley-Milner, see
[[2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive]]), `lower.rs` (shape-preserving map
to `Mir`, threading the solved type forward), `fold.rs` (the one compile-time evaluator, see
[[2026-07-09-const-folding-is-the-one-tier-poison-plus-dce-give-reachability]]), `layout.rs` (fix the
whole boundary surface and every function's absolute index once), `select.rs` (`Mir → Lir` instruction
selection), `serialize.rs` (pure byte-laying). Two invariants make the ladder hold its shape: every rung
(`Hir`, `Ty`, `Mir`, `Lir`) is a **typed sum, and every pass matches it exhaustively**, so a variant a
pass does not yet handle is a *compile error in the compiler itself*, never a silent fall-through; and the
IR stays **concrete** downward — `select` resolves every call to an absolute wasm function index (`Lir`
carries `Call(index)`, not `CallImport` plus a serialize-time remap), because the earlier fused compiler's
bugs lived precisely in the remaps and re-derivations that a "convenient" late-binding IR invites.

The desugarings that collapse surface distinctions all happen at `resolve`, the top rung, so no lower pass
ever sees them: `and`/`or`/`not` become `Hir::If`; a value-def becomes a nullary function; a `do` block
becomes a `let` chain; a module becomes a value-def bound to a record of its exports (see
[[2026-07-09-everything-is-a-record-nothing-built-in-is-privileged-by-name]]). By `Mir`, "named vs
positional," "record vs tuple," and "pattern vs constructing expression" are gone — one `Mir::Proj { slot,
elem_ty, operand }` reads both a tuple (literal slot) and a record (name→slot resolved at lowering), and a
pattern is an ordinary expression tree with two extra leaves (`Local` = binder, `Wildcard`).

**Why.** The predecessor was a single `emit(node) -> (bytes, Kind)` walk that fused five jobs — reject,
fold, inline, resolve-the-handler, append-bytes — and its failures were *properties of doing analysis
during emission*, not of any feature: an order-dependent handler stack that miscompiled nested same-effect
handlers, and exponential re-emission on every branch. The resolved-IR learning fixed the *principle* (a
middle rung exists; emission only serializes) but stopped short of committing to a decomposition, on the
correct instinct that the spec should not prescribe a pass list. Authoring the whole compiler proved the
converse for the *implementation*: the reason the fused walk could not be incrementally repaired is that it
had **no rung at which a single concern was the only concern**, so every fix touched every other. A
nanopass ladder is the structural cure — a concern that has exactly one home cannot leak into the others,
and a rung that is an exhaustively-matched sum turns "we forgot to handle this construct" from a runtime
miscompile into a compiler that will not build. The concreteness rule (absolute indices, no late remap) is
the same instinct applied to the *bottom*: every place a value is re-derived or re-bound downstream of the
decision that fixed it is a place the two derivations can disagree, which is the entire failure mode the
rebuild was undertaken to eliminate ([[2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point]]).

The spec is right to name only the *obligations* — an AST-valued input, a resolved analyzed middle, a typed
instruction sum serialized by an exhaustive match — and to leave the ladder to the implementation
(`compiler-pipeline.md` §Purpose: "it does not prescribe the phase decomposition beyond requiring that one
exist and be respected"). But "a solid pipeline is rediscovered from the obligations" is exactly the
hand-holding the rebuild showed a fresh implementer cannot skip: the obligations admit the fused walk as
much as the ladder, and the fused walk is where the bugs were. This learning records the *rediscovered*
decomposition and its two invariants so that the next implementation starts from the shape that works
rather than re-deriving it through the same class of miscompiles.

**The requirement it drove.** No new *behavioral* requirement — the ladder is discharged, like the sibling
§Representation requirements, by the requirement gate (an implementation and a test citation), and it
realizes the standing `spec/capabilities/compiler-pipeline.md` §"The Compiler Resolves Names Before It
Selects Instructions," §"Emission Serializes A Lowered Representation," §"The Compiler Operates On AST
Values" (a typed instruction sum serialized by exhaustive match), and §"The Pipeline Has Defined Phases."
The reproduction content that is **not yet folded** and should become the seed of the architecture
reference doc: (1) the *exhaustive-match invariant* — a rung is a typed sum and an unhandled variant is a
compiler-build error, generalizing the existing serializer-exhaustiveness requirement from the last rung to
every rung; and (2) the *keep-the-IR-concrete* rule — a decision, once fixed by a pass (a call's target
index, a field's slot), is carried concretely and never re-derived or remapped by a later pass. Both are
architecture, not language semantics, and belong in the prescriptive architecture doc rather than in an
implementation-free capability spec.
