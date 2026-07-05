# Macros are typed (Expr[T] over Ast) and hygienic (sets-of-scopes carried in the AST)

*2026-07-04*

**What happened.** The metaprogramming surface gains two commitments the first cut left as bare
assertions: **typed quotes** and a **concrete hygiene mechanism**.

- **Typed construction over untyped analysis.** The untyped `Ast` sum type that `quote` produces today
  stays the substrate for *analyzing* arbitrary input code (the compiler walks a program it did not
  write — [[2026-07-03-ast-construction-vs-ast-evaluation]]). Layered over it, a **typed quote** is an
  `Expr[T]` — a fragment of code that *produces a `T`*. A macro that assembles an ill-typed fragment is
  rejected **at the macro definition**, not when its expansion later fails to type-check somewhere in
  generated code. `Ast` for analysis; `Expr[T]` for hygienic, typed construction — both, layered, the
  same way nominal types layer over structural records
  ([[2026-07-04-nominal-is-orthogonal-tag-over-structural-types]]).
- **Sets-of-scopes hygiene.** Hygiene (`metaprogramming.md` §"Macros Are Hygienic") is realized by
  Racket's **set-of-scopes** model: every identifier carries the set of scopes in force where it was
  introduced, and name resolution compares scope sets so a macro-introduced `x` and a use-site `x` are
  distinct bindings unless the macro explicitly requests capture.

**Why.**
- **The static spine forces typed quotes.** The language has committed that **static typing is
  mandatory** ([[2026-07-04-static-typing-is-mandatory-post-pivot]]) and that **Cadenza source is
  authored as static even where the seed is dynamic**
  ([[2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic]]). Untyped-only macros
  contradict that: they defer all checking to post-expansion, producing errors in code the author never
  wrote. **Scala 3** (`Expr[T]`/`quoted`) and **Typed Template Haskell** (`Code Q a`) both *started*
  untyped and *moved* to typed quotes for exactly this reason. For an **agent-authored** language the
  payoff is decisive: a macro whose output is checked at definition gives the agent a machine-readable
  rejection *at the macro* (Constitution XI), not a mysterious downstream error. Typed quotes are the
  construction analogue of the analysis substrate — they do not replace `Ast` matching (`(quote 42)`
  still matches `Ast.Int`), they sit above it.
- **Hygiene cannot ship as an assertion.** "A name a macro introduces MUST NOT capture a use-site name"
  is not a mechanism — it is a property that only holds if identifiers carry enough information for the
  resolver to *tell macro-introduced names apart*. The design already had the hook: the glossary says
  "a macro introduces symbols in its own namespace so the two cannot collide," and `ast-encoding.md`
  already namespaces every symbol. Sets-of-scopes generalizes that hook into a real, well-understood
  algorithm; `syntax-case` (marks/antimarks) is the older alternative, rejected as more intricate.

**The frozen-contract consequence — an ADDITIVE `ast-encoding.md` extension (operator-approved to
enact).** Hygiene requires an identifier node to carry **scope-set** information beyond its
namespaced-symbol reference. That is a change to the frozen AST-encoding contract — but an **additive**
one, and the contract already provides for exactly this: §"New Constructs Do Not Bump The Encoding
Version" and §"The Encoding Is General And Stable" (a new symbol/attribute does not change the encoding
of a tree that does not reference it). So a program with no macros encodes identically to today; only
identifiers that carry scope information use the new attribute. The operator approved **enacting** this
additive edit in the requirement pass (not merely proposing it), under the contract's own additive-
evolution rule — no container-version bump, no migration path required because already-stored trees are
unaffected. Typed `Expr[T]` needs **no** encoding change: it is a compile-time typing over the same
`Ast` container, erased like every other type ([[2026-07-04-static-typing-is-mandatory-post-pivot]]).

**Consequences.**
- **A macro is a typed compile-time function.** It runs in the one compile-time tier
  ([[2026-07-04-compile-time-evaluation-is-one-tier]]), takes/returns `Ast` (or `Expr[T]` for typed
  construction), and its own body is type-checked like any function — so `Expr[T]` composition errors
  are caught at macro-definition time.
- **Hygiene interacts with modules, not just lexical scope.** A macro-introduced reference to a
  top-level binding resolves at the *macro definition's* scope, so a macro can refer to its own module's
  helpers without the use site needing them imported — the module analogue of lexical hygiene, and why
  the phase model must say which definitions are available at expansion
  ([[2026-07-04-macro-phases-and-the-reader-stays-fixed]]).
- **Explicit capture stays possible.** The requirement's "unless the macro explicitly requests it"
  survives: an explicitly unhygienic (use-site-scoped) identifier is a deliberate operation over scope
  sets, not the default.

**The requirements it drives.** `spec/capabilities/metaprogramming.md` §"Macros Are Hygienic" is
sharpened from an assertion to name the **set-of-scopes** model (identifiers carry scope sets; resolution
compares them; explicit capture is an explicit scope operation), and a new §"Typed Quotes Construct
Well-Typed Fragments" adds `Expr[T]` over the untyped `Ast` substrate (a macro building an ill-typed
fragment is rejected at definition; `Ast` remains the analysis substrate). `spec/contracts/ast-encoding.md`
gains an **additive** requirement that an identifier node MAY carry a scope-set attribute for hygiene,
expressible as a new symbol/attribute without changing the encoding of a tree that does not reference it
(no container-version bump). Composes with [[2026-07-04-compile-time-evaluation-is-one-tier]] and
[[2026-07-04-macro-phases-and-the-reader-stays-fixed]].
