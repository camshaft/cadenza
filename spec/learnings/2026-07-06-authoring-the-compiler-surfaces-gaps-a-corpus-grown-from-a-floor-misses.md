# Authoring the compiler in Cadenza surfaces gaps a corpus grown from a floor misses

*2026-07-06*

**What happened.** With effects landed in the seed, the compiler was (re-)authored *in Cadenza* — a
resolved IR ladder (`Core` → `Lir` typed instruction sum → bytes) whose emission is a pure
serializer, per `compiler-pipeline.md` §Representation. Written idiomatically — recursive
tree-walking passes over sum-typed IR, `match` arms returning bytes, factored accessors — the
vertical slice reached a working end-to-end pipeline: the Cadenza compiler compiled
`(module m (def (main) (+ 20 22)))` to a valid WebAssembly component whose code section is
`i64.const 20 · i64.const 22 · i64.add · end`, which runs to 42. Getting there surfaced four seed/spec
gaps that months of isolated conformance cases had not:

1. **Compile-time inlining was exponential.** A recursive value function consuming a compound
   argument was inlined at compile time and its argument re-expanded at every reference; the idiomatic
   compiler drove this past 30 GB before the OS killed it. The minimal reproducer was one constructor
   consumed by one recursive `match`. Root cause (found on the fix): a recursive consumer that
   *threads a compound accumulator* had that parameter's kind inferred as a scalar, so a heap argument
   met a scalar parameter, the polymorphic-call path fell into inlining, and the recursion inlined
   without bound. Fixed in the seed's kind inference — back-propagate a `match`'s unified result kind
   to arms that merely return a parameter, and let the "more defined" heap kind win an
   order-dependent constraint race — so the recursive call lowers to a real `call`, not an inline.
2. **No boolean connectives** (`and`/`or`/`not`) existed anywhere — seed, corpus, or spec — captured
   separately in [the boolean-connectives learning](./2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives.md).
3. **Runtime `String` is unrealized**, which walls off name-based dispatch and the reader's symbol
   table — the keystone remaining blocker for a self-hosting front end (the built-in `Ast` type is
   const-fold-only, so the Cadenza compiler must decode the canonical binary AST into its own sum).
4. **A `match` arm returning a heap value bound by its own pattern, through a called helper, could
   emit an invalid component** rather than declining — a "decline, don't miscompile" violation on a
   tree-walker's hot path.

**Why.** The conformance corpus is grown outward from the mandatory floor, case by case; each case
witnesses one requirement in isolation. That process is excellent at deepening coverage of a feature
it has decided to exercise and structurally blind to the *interactions and staples* a real program
composes — a recursive pass that threads an accumulator, a two-condition predicate, a helper that
returns a sub-node. An isolated case never needed a boolean connective, never threaded a compound
accumulator through a recursive call, so those gaps sat undisturbed. Writing the compiler is a
different kind of pressure: it is one program that uses the whole language at once, in the shapes real
code takes, so it finds the cracks *between* individually-correct features. This is the concrete
payoff of the two-compilers architecture beyond judgment-independence — authoring the second compiler
is itself the most demanding conformance test the language has, and it earns its keep as a
gap-finder long before it is self-hosting.

**The requirement it drove.** Directly: the boolean-connectives requirement in
`core-semantics.md` (its own learning). Indirectly but durably: the exponential-inlining fix is
guarded by a new conformance case in `05-compound-types.sexp` (*"a recursive sum consumer whose
arguments are recursive sum producers compiles"*), turning a compiler-crashing shape into a permanent
gate obligation; and the parameterized-entry bug (`(def (main n) …)` emitting an invalid component)
now declines cleanly (*"the entrypoint `main` must take no parameters"*). The remaining gaps —
runtime `String`, and the heap-sub-node-through-a-helper miscompile — are recorded with reproducers
in the compiler spike's handoff so they become the next round of seed work rather than lore. The
methodological lesson stands on its own: **grow the corpus from a floor, but validate it by authoring
a whole program**, because the staples a program leans on are exactly what a floor-outward corpus
forgets to require.
