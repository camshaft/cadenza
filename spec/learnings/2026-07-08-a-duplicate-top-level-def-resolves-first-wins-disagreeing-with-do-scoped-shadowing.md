# A duplicate top-level def resolves first-wins, disagreeing with do-scoped shadowing

*2026-07-08*

**What happened.** Adversarial probing of the definition scope found that a duplicate top-level
`def` is silently accepted and resolved by **implicit precedence — first-wins** — a rule that
matches no spec interpretation and that *disagrees with how the same construct resolves in a `do`
block*:

- Top level: `(module m (def (f) 1) (def (f) 2) (def (main) (f)))` → `1` (the FIRST definition
  wins; the second is silently dropped). `(def (f x) x)` followed by `(def (f x y) …)` likewise
  keeps the first — `(f 5)` = 5, and `(f 5 6)` is rejected as over-applying the one-parameter
  first `f`, proving the two-parameter second `def` was discarded.
- `do` block: `(do (def x 1) (def x 2) x)` → `2` (the LAST definition wins — sequential
  shadowing, per core-semantics.md §"A Declaration In A Sequencing Block Is Scoped To The Forms
  That Follow It").

So the identical duplicate-`def` construct resolves first-wins at module top level but last-wins
in a `do` block.

**Why it is a defect.** Two readings of a duplicate top-level `def` are defensible, and the
compiler's behavior matches *neither*:
- **Reject** — core-semantics.md §"A Module Evaluates To A Record Of Its Exports" says each
  definition registers its name and value as a field of the module's record, and a record has a
  fixed set of named fields (a duplicate field is CDZ0201). Two same-named defs → a duplicate
  export field → ill-formed. modules-and-namespaces.md §"Colliding Imported Names Are Rejected"
  states the adjacent principle for imports: two definitions under one name "MUST be a
  compile-time error rather than resolved by an implicit precedence."
- **Last-wins shadowing** — if a top-level `def` binds sequentially like a `let` binding or a
  `do`-scoped `def` (both of which shadow last), a redefinition would take effect for what
  follows, giving `2`.

First-wins is the one answer both readings rule out: the record reading says reject, the
sequential reading says last. The top-level definition scope is the only binding scope in the
language that resolves a repeated name by first-wins — `let` shadows last (02-binding
§"a repeated let binding shadows the earlier one"), a pattern binding the same name twice is
CDZ0102, a `do`-scoped `def` shadows last. Top-level `def` is the outlier, and its rule is
unspecified.

**Where the spec is silent.** modules-and-namespaces.md pins colliding *imported* names (reject,
no implicit precedence) but says nothing about two `def`s of the same name *within one module*.
The module-is-a-record chain implies reject, but no normative sentence states it directly, and
no diagnostic code is assigned. So the outcome is genuinely unspecified — which is itself the
gap: a rule the spec states for imports (no implicit precedence) is left unstated for the
structurally identical local case, and the implementation fills the gap with a third rule
(first-wins) that contradicts both the import rule and the sequential-shadowing rule it uses
everywhere else.

**The lesson.** When a spec pins a resolution rule for one form of name collision (imports) and
leaves the sibling form (local defs) unstated, an implementation will fill the gap — and it may
fill it inconsistently with both the stated rule and its own behavior elsewhere. The tell here
was that two scopes for the *same* declaration form (`def` at top level vs in `do`) resolve a
duplicate oppositely; a single declaration form should have one shadowing/collision rule across
every scope it appears in. The spec should state the top-level rule explicitly — either
"a module rejects two definitions under one name (CDZ0201, the duplicate-export-field of
module-is-a-record)" or "top-level definitions shadow sequentially like `do`-scoped ones" — and
the corpus should pin it; today it is unpinned, so no gate case is added (probing an unspecified
point yields a learning, not an invented oracle).

**Status.** No corpus case added — the outcome (reject vs last-wins) is not spec-fixed, so pinning
one would invent an oracle. Recorded as this learning plus a note to resolve the spec gap. The
duplicate-effect-declaration variant is the same shape and *does* cause a concrete false-reject:
`(effect E (op a …)) (effect E (op b …))` keeps the LAST `E`, so a valid `E.a` reference is
rejected "effect `E` does not declare `a`" — effect declarations resolve last-wins where top-level
`def` resolves first-wins, another facet of the same unspecified-collision gap. Native seed.
