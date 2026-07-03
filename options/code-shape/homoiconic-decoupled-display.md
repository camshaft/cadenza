# Code Shape — Choice: homoiconic-decoupled-display

> **A choice for the `code-shape` decision** (see [README.md](./README.md) for the decision and the
> requirements a choice must satisfy). This is the **default** choice. It is a declared choice, not a
> requirement; the whole specification is written against "the canonical representation" and "the
> binary AST," so adopting a different choice touches no frozen contract and no capability
> requirement.

## The insight: homoiconicity decouples display from representation

If the canonical representation is **homoiconic** — the program *is* a uniform data structure, code
as data — then display and representation come apart cleanly. The representation is the one durable,
hashable, manipulable thing; a *display* is any deterministic rendering of it. There is no single
"the syntax" that both humans and agents and the hash must agree on; there is one representation and
as many displays as are useful, each a projection. This is a better answer than picking one surface
and taxing whichever north-star priority it serves worst, because it serves all of them at once
through different projections of the same core.

## The choice: a homoiconic canonical representation with decoupled displays

The **canonical representation is a homoiconic, typed term**, and its **canonical stored form is the
binary AST** (ast-encoding.md) — the uniform code-as-data structure that is content-addressed and is
the sole target of structural manipulation, hashing, the executable semantics, and verification.
**Text is decoupled from it:** a program is *stored* as the binary AST and merely *shown* through a
textual syntax, and more than one textual syntax may exist —

- a **conventional syntax** in the ML/Rust family (expression-oriented, keyword- and brace-delimited,
  indentation-insensitive) for humans to read and write comfortably;
- an **s-expression syntax**, the direct code-as-data rendering, for metaprogramming and for agents
  that manipulate structure literally;
- any further syntax a deployment adds, as a parser and printer to and from the binary AST.

No textual syntax is the stored form; each is a lossless projection that parses to and prints from the
binary AST, so moving between syntaxes never changes the program. The one canonical byte form is the
binary AST, not any rendering, which is what the constitution's round-trip requires.

## The two displays, shown

The same program — a documented function, a match, and a module with a capability declaration — in
both displays. Both project losslessly to the one representation.

**Homoiconic display** (the direct code-as-data rendering; also the corpus form):

```
(module math
  (doc "Small integer helpers.")
  (use (capability emit-event))
  (def (classify n)
    (doc "Sign of n as a tag.")
    (: (-> Int64 Sign))
    (match n
      ((< n 0) Sign.Neg)
      ((= n 0) Sign.Zero)
      (else    Sign.Pos))))
```

**Conventional display** (a projection of the very same representation):

```
module math

/// Small integer helpers.
use capability emit-event

/// Sign of n as a tag.
fn classify(n: Int64) -> Sign =
  match n {
    n < 0 => Sign.Neg
    n = 0 => Sign.Zero
    else  => Sign.Pos
  }
```

Documentation is a node in the representation (the `doc` form / `///` projection), not lexical
trivia, so it survives the round-trip in either direction (agent-authoring.md §Documentation).
A comment is likewise a node (the `comment` form / `//` projection), attached to the part it
annotates and parsed into the tree rather than dropped, so it too survives the round-trip and is
stored in the binary AST — the canonical stored form is the tree, so a comment the parser discarded
would not be stored at all (agent-authoring.md §Comments; ast-encoding.md §"The Tree Carries
Comments And Documentation").

## Why this choice, against the north star

- **Written by agents (priority #1):** because code is data, an agent produces and transforms a
  program by manipulating the homoiconic representation directly through the structural interface —
  the strongest form of "easy to write," independent of any display's whitespace or delimiters.
- **Read by agents and humans (priorities #1 and #2):** display is decoupled, so humans read the
  conventional display while agents may read either; neither priority is sacrificed to the other,
  because they are different projections of one representation rather than one contested syntax.
- **Verify properties (priority #3):** verification, the type system, and the executable-semantics
  corpus all operate on the uniform homoiconic representation — the property a homoiconic core is
  prized for — while humans still get a conventional display.
- **Reproducible codegen:** the hash and the round-trip are defined against the binary AST, the one
  canonical byte form; because identity is the tree and not any rendering, whitespace, line endings,
  and choice of syntax cannot affect a program's identity, and the byte-identical round-trip the
  constitution requires is immediate.

## The core symbol set (the ignition surface)

A node names its kind by referencing a symbol in the `cadenza/core` namespace (ast-encoding.md
§"A Prelude Symbol Is Namespaced And May Be Versioned"; `options/ast-encoding/binary-sexpr.md`). The
meaning of each symbol is the executable-semantics corpus; this table pins the **minimal core symbol
set the ignition corpus references**, so that every head symbol in a `spec/semantics/*.sexp` case
resolves to a named core construct rather than an invented one. It is a code-shape choice (the
representation's surface), not a frozen contract; a later generation adds symbols without touching any
contract, exactly as adding a construct adds a prelude symbol rather than bumping the container version.

| Symbol (`cadenza/core`) | Arity / shape | Construct it names |
|---|---|---|
| `module` | `(module <name> <form>…)` | a module: a named unit of definitions and capability declarations |
| `def` | `(def (<name> <param>…) <form>…)` | a definition (value or function) |
| `doc` | `(doc "<text>")` | documentation attached to the enclosing definition/module (a node, not trivia) |
| `comment` | `(comment "<text>" <annotated>)` | a human comment attached to the node it annotates (a node, not trivia); semantically inert |
| `use` | `(use (capability <cap>))` | a capability declaration, contributing `<cap>` to the module's manifest |
| `capability` | `(capability <cap-name>)` | names a host capability inside a `use` |
| `:` | `(: <expr> <Type>)` / `(: (-> <T>… <R>))` | a type annotation; also the corpus value-form head `(: <value> <Type>)` |
| `->` | `(-> <T>… <R>)` | a function type |
| `let` | `(let ((<name> <expr>)…) <body>)` | a lexical binding form |
| `fn` | `(fn (<param>…) <body>)` | a function value (lambda): captures its enclosing scope, applied by `(<fn-expr> <arg>…)` |
| `if` | `(if <cond> <then> <else>)` | a two-branch conditional; evaluates only the selected branch |
| `match` | `(match <scrutinee> (<pattern> <body>)…)` | pattern matching, governed by the exhaustiveness rule |
| `else` | `else` (a match pattern) | the catch-all match pattern |
| `record` | `(record (<field> <expr>)…)` | a structural record constructor |
| `list` | `(list <expr>…)` | a list literal |
| `map` | `(map (<key> <value>)…)` | a map literal |
| `=` | `(= <a> <b>)` | structural-equality comparison |
| `+` `+%` | `(+ <a> <b>)` / `(+% <a> <b>)` | checked addition; wrapping addition (distinct wrapping type) |
| `unit` | `unit` | the unit value, the normal-termination value of an effect-only program (e.g. one whose `main` only emits events); its type is `Unit` |
| field access | `<expr>.<field>` | record/nominal field projection (e.g. `p.x`) |

**Function application** is written `(<fn-expr> <arg>…)` where the head is an *expression that
evaluates to a function* (a name bound to a `fn`, a `def`, or an inline `(fn …)`), rather than a
`cadenza/core` construct symbol. This is how a program applies a first-class function value
(core-semantics.md §"A Function Is A First-Class Value", §"Applying A Function Binds Its Parameters To
Its Arguments"); a top-level `(def (<name> <param>…) <body>…)` is sugar for binding `<name>` to a
`(fn (<param>…) <body>…)`. A head that resolves to a core construct symbol names that construct; a head
that resolves to a bound function value applies it.

Names, sum-type variants, and the numeric/collection operations a program calls (`Sign.Neg`,
`Some`/`None`, `List.at`, `Rational.of`, `Float64.of-int`, `Int64.max`, `Wrapping64.max`, the built-in
type names `Int64`/`Float64`/`Bool`/`String`/`Rational`/`Wrapping64`/`Unit`) are ordinary bound names and
constructors resolved by their declarations, not additional core syntax; the corpus grounds each where
it is used. The floating-point not-a-number literal is written `nan` and denotes the canonical
not-a-number value (deterministic-value-form.md §"Numeric Values Serialize Deterministically"). The
unit value is written `unit` and is the sole value of the `Unit` type — the normal-termination value of
a program that produces no value other than through its emitted events (deterministic-value-form.md
§"The Unit Value Has A Canonical Byte Form"; core-semantics.md §"An Effect-Only Expression Yields The
Unit Value").

Sum types the ignition corpus uses are declared where the corpus references them: `Sign` as
`(Neg | Zero | Pos)` (nullary variants), and an `Option` with a payload-carrying variant as
`(Some <value> | None)`, so a pattern that binds a variant's payload (`(Some n)`) has a grounded
declaration (core-semantics.md §"Bindings Introduced By A Pattern Are Scoped To Its Branch").

## What this choice fixes vs. leaves to the spec

- **Fixed by the spec (requirements):** that a canonical form exists and round-trips, and that a
  structural interface exists. These hold regardless of representation or display.
- **Fixed by this choice (replaceable):** that the canonical representation is homoiconic, that
  display is a decoupled projection, the set of displays offered, and which display is the canonical
  textual form. Adding, removing, or changing a display is a change to this choice and its
  projection; it touches no contract and no capability requirement, precisely because display is
  decoupled from representation.
