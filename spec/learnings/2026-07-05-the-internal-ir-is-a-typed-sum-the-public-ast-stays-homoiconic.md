# The compiler's internal IR is a typed sum; the public AST stays homoiconic

*2026-07-05*

**What happened.** compiler-pipeline.md mandates that the compiler "emit instructions as AST sum type
values, not as string-tagged pseudo-structures," constructed via quasiquote and serialized by a
recursive function over the AST — witnessed by the corpus case that builds `` `(op-const ,n) ``. Authoring
the compiler in Cadenza ([[2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps]])
showed that following this *to the letter* undercuts its own stated goal, and two related decisions
resolved it:

- **The backend IR is a dedicated typed sum, e.g. `(type Instr (Const Int64 | Add | LocalGet (UInt 32)
  | Call (UInt 32) | End))`, not `Ast` nodes.** The serializer pattern-matches that sum
  (`(match instr ((Instr.Const n) …) ((Instr.Add) …) …)`), so adding an opcode is a **compile error until
  it is handled** — exhaustiveness (core-semantics.md §"Matching Is Exhaustive Or Rejected") makes the
  backend safe by construction.

- **Symbols belong in the internal IR, not the public AST.** The compiler keys its symbol table and
  internal terms on interned `Symbol`s for O(1) name comparison
  ([[2026-07-05-self-hosting-is-gated-on-generics-the-rest-is-libraries-and-scale]] Tier-4 win), but the
  homoiconic `Ast` that `quote` produces keeps `String`-named identifiers: `(quote foo)` is
  `(Ast.Name "foo")`, load-bearing for the whole quote/macro surface and its structural-equality
  corpus. Interning happens at the **`Ast → internal-term` lowering boundary**, seeded by the symbol
  table the binary AST encoding already carries (names are stored as indices into a symbol section), so
  it is O(unique names), and — because a `Symbol`'s identity is content-derived, never allocation-order
  — interning an externally-supplied AST eagerly, lazily, or not at all are the same result, a
  performance choice rather than a correctness one.

**Why.** The literal reading defeats the requirement's intent:

- **`(Ast.List (list (Ast.Name "i64-const") …))` *is* a string tag** — the head `"i64-const"` is a
  `String` in a `Name` payload. The spec asked for "not string-tagged pseudo-structures"; a stringly-keyed
  `Ast` serializer that must fall through an `else` arm on an unknown head is precisely a string-tagged
  pseudo-structure. A typed `Instr` sum serves the *intent* better than the letter does, and it extends
  the project's central discipline, "decline, do not miscompile"
  ([[2026-07-03-decline-do-not-miscompile]]), into the backend: an unhandled instruction cannot compile,
  so it can never silently miscompile. It also opens the door to const-folding and peephole
  optimizations that pattern-match structurally — brittle and error-prone over open-headed `Ast` nodes,
  natural over a closed typed sum.

- **The pipeline has two honest halves.** The *frontend* is unavoidably homoiconic — decode to `Ast`,
  walk `Ast`, and (for a macro layer) quasiquote `Ast` — because there the values genuinely *are* the
  program's syntax as data ([[2026-07-03-quasiquote-for-programmatic-ast-construction]],
  [[2026-07-03-ast-construction-vs-ast-evaluation]]). The *backend* reasons about instructions and types
  the program never sees as source. Quasiquote does not disappear; it moves to where the values are truly
  `Ast`. This mirrors the standing split between the homoiconic canonical representation and the typed
  things built over it, and complements the rule that Cadenza source is written statically-typed even
  under a permissive seed ([[2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic]]).

- **Keeping `Symbol` out of the public `Ast`** preserves the one AST value form the encoding is a
  bijection over: `(quote foo)` must equal `(Ast.Name "foo")` and encode identically however built. A
  `Symbol`-carrying `Ast.Name` would split that value form and break the quote-vs-constructor equality
  the metaprogramming corpus pins. The internal term is a *distinct* representation the compiler lowers
  *to*, so interning there costs the public surface nothing.

**The requirement it drove.** `compiler-pipeline.md` §Representation was amended (operator-ratified
2026-07-05). §"The Compiler Operates On AST Values" now requires instructions to be represented as a
**typed sum type — the AST sum *or* a dedicated instruction sum** — deconstructible by pattern matching
and not string-tagged, and requires the serializer to pattern-match that sum **exhaustively**, so an
unhandled instruction variant is a compile-time error rather than a silent fall-through — the "not
string-tagged" goal met by types rather than by a naming convention. The quasiquote requirement was
re-scoped and its heading renamed to §"The Compiler Constructs AST Values Via Quasiquote": quasiquote is
required in the frontend and macro layer, where the values built are program syntax, while a dedicated
instruction sum is built by ordinary constructors and matched to bytes. The mirrored `.duvet/`
requirement quotes were updated to match (the renamed heading's requirement file was renamed to its new
slug), and the metaprogramming corpus case "quasiquote makes AST construction readable" re-cites the
renamed heading. This also records that the self-hosting compiler interns names into `Symbol`s only in
its internal representation, at the `Ast → term` boundary, leaving the quotable `Ast` `String`-named
([[2026-07-05-self-hosting-is-gated-on-generics-the-rest-is-libraries-and-scale]]). No frozen contract is
touched: the component ABI already forbids generics at the boundary and says nothing about a compiler's
internal representation, which it is explicitly free to choose.
