# The built-in list cannot be pattern-matched — the biggest ergonomic gap for authoring the compiler

*2026-07-07*

**What happened.** As the reader grew (recursive CBOR decode that walks arrays of child items), the
spike hit a gap that shapes *every* list-consuming pass in the compiler: **the built-in `list` cannot
be pattern-matched at all.** Every list-pattern form declines — `(cons h t)`/`nil`, positional
`(list a b c)`, and empty `(list)` all give "unsupported list pattern"; `(List.Cons (tuple h t))`
gives "runtime sum match on an undeclared variant". A `list` is consume-only: `List.at` (→ `Option`)
+ `List.len` + index recursion. And `core-semantics.md` §Pattern Matching covers tuples and sum
constructors but says **nothing about lists** — so this is unspecified as well as unimplemented.

The consequence is visible throughout `compiler.cdz`: every place that folds a sequence — the module's
def list, a function's code stream, a CBOR array's children — is hand-rolled as a **custom cons-sum**
(`(type FList (FNil | FCons (Tuple Func FList)))`, and likewise `Code`, `DList`), each one
re-implementing the persistent sequence the language already has, purely so it can be `match`ed. The
corpus shows the same tell: it folds a *user* `IntList` sum, never the built-in `list`, precisely
because the built-in cannot be matched (`05-compound-types.sexp` "a recursive function folds a runtime
linked list to a scalar"). This is the single biggest ergonomic gap remaining for authoring the
compiler idiomatically — not a blocker (the cons-sum workaround compiles and runs) but a steady tax
that duplicates the sequence type at every use.

**Why.** The right design keeps the list's representation *opaque* — a `list` is a persistent tree,
not a cons list, so exposing `Cons`/`Nil` cells (Lisp-style) would leak an internal representation the
language deliberately hides and that the runtime does not even use. The natural fit is **ML/Rust-style
element patterns with a rest binder**: `(list)` matches exactly the empty list, `(list x .. rest)`
binds the first element and the remaining elements *as a list*, and fixed-arity `(list x y)` is sugar
for a length check. An exhaustive fold needs only the empty case and a rest-pattern case; the matcher
asks `len`/`first`/`rest` (all expressible over `List.at`/`List.len`/`List.slice`, which exist), so the
representation stays hidden while the structural fold becomes natural. This matters beyond ergonomics
because it is a *spec* decision, not just a seed feature: pattern matching is a core-semantics surface,
and "how is a list deconstructed" is a hole in that surface that the compiler-authoring pressure
exposed — the same way authoring the compiler surfaced the absence of boolean connectives
([[2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives]]) and nested payload
binders. A floor-outward corpus never required it because no isolated case needs to fold the built-in
`list` structurally; a whole program that folds sequences everywhere needs it constantly. Once
specified and realized, the compiler's `Code`/`FList`/`DList` cons-sums collapse to the built-in `list`
with a natural `match` — a real simplification of the compiler, not just nicer syntax.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"the built-in list is
folded by an element-with-rest pattern"* (`(match xs ((list) 0) ((list x .. rest) (+ x (sum rest))))`,
`sum (list 10 20 30) → 60`) — pins the proposed design's core: the empty arm and the element-plus-rest
arm that together make a fold total, over the built-in `list` with its representation opaque. It
records the true oracle (60) and is tagged `(needs list-patterns)` — an unrealized capability — so it
**skips** until a generation specifies and realizes list deconstruction (distinct from a `todo`
decline: element-with-rest patterns are a new capability to add, not a rule the seed should already
cover). The spec change it calls for is a new `core-semantics.md` §Pattern Matching clause — *"A List
Is Deconstructed By Element Patterns With An Optional Rest"* — recorded as SPEC-BACKLOG item 13 for the
operator, since it is a language-surface addition (a MUST about how `match` sees a list), not merely
seed work. Related reader progress: `String.from-bytes`-through-a-boundary (the symbol table) was
reclassified by the spike as NOT blocking the reader — the reader decodes raw bytes with `Bytes.at`,
and only prelude-symbol name comparison needs `from-bytes` — so list patterns, not `from-bytes`, are
now the compiler's largest idiomatic gap.
