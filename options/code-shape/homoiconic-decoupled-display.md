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
  (effect emit-event (op emit (-> String Unit)))
  (def (classify n)
    (doc "Sign of n as a tag.")
    (: (-> Int64 Sign))
    (if (< n 0) (Sign.Neg unit)
        (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))))
  (def (describe s)
    (doc "Name a sign.")
    (: (-> Sign String))
    (match s
      ((Sign.Neg _)  "negative")
      ((Sign.Zero _) "zero")
      ((Sign.Pos _)  "positive"))))
```

**Conventional display** (a projection of the very same representation):

```
module math

/// Small integer helpers.
effect emit-event { op emit(String) -> Unit } host

/// Sign of n as a tag.
fn classify(n: Int64) -> Sign =
  if n < 0 then Sign.Neg(unit)
  else if n = 0 then Sign.Zero(unit)
  else Sign.Pos(unit)

/// Name a sign.
fn describe(s: Sign) -> String =
  match s {
    Sign.Neg(_)  => "negative"
    Sign.Zero(_) => "zero"
    Sign.Pos(_)  => "positive"
  }
```

The two definitions show a division of labor the surface makes visible: a *value* condition (is `n`
negative?) is an `if`, while `match` is used **only** to destructure a value's variants. A match arm's
head is a **pattern** — a constructor that destructures, a literal, a binding, or the `_`/`else`
catch-all — never an arbitrary boolean predicate, so `match` is not a `cond` in disguise and the set of
arms can be checked against the scrutinee's type for exhaustiveness. Value-level refinement of a pattern
(a `pattern if guard` clause) is a separate, optional concern deliberately kept out of the arm head, and
a guard, were one added, would not count toward exhaustiveness.

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
| `module` | `(module <name> <form>…)` | a module: binds `<name>` in the enclosing scope to a **record** of its exported definitions, carrying its capability manifest and entry as metadata |
| `def` | `(def (<name> <param>…) <form>…)` | a definition that registers a named export into the enclosing module's record |
| `doc` | `(doc "<text>")` | documentation attached to the enclosing definition/module (a node, not trivia) |
| `comment` | `(comment "<text>" <annotated>)` | a human comment attached to the node it annotates (a node, not trivia); semantically inert |
| `effect` | `(effect <name> (op <op> <type>)…)` | declares an effect and types each of its operations — a routing-agnostic contract that says nothing about where the effect is discharged (routing is decided by an enclosing `handle` or entrypoint `host`) |
| `op` | `(op <name> (-> <T>… <R>))` | names and types one operation of an effect, inside an `(effect …)` |
| `host` | `(host (<effect>…) <body>)` | an entrypoint delegation: within `<body>`, routes the named effects to the component boundary as plain imported-function calls the host resolves, making the host their terminal handler and enumerating them in the manifest (the delegation is the grant — there is no separate capability form) |
| `handle` | `(handle ((<Effect>.<op> (<param>…) <body>)…) <body>)` | discharges an effect in-program; a `<body>` may `resume` the continuation at most once; a `handle` nearer a perform than an enclosing `host` interposes on an otherwise-delegated effect |
| `:` | `(: <expr> <Type>)` / `(: (-> <T>… <R>))` | a type annotation; also the corpus value-form head `(: <value> <Type>)` |
| `->` | `(-> <T>… <R>)` | a function type |
| `let` | `(let ((<name> <expr>)…) <body>)` | a lexical binding form |
| `do` | `(do <form>…)` | a sequencing block: evaluates each form in order and yields the last form's value; a declaration form (`module`, `def`) binds its name for the forms that follow it in the block |
| `fn` | `(fn (<param>…) <body>)` | a function value (lambda): captures its enclosing scope, applied by `(<fn-expr> <arg>…)` |
| `if` | `(if <cond> <then> <else>)` | a two-branch conditional; evaluates only the selected branch |
| `match` | `(match <scrutinee> (<pattern> <body>)…)` | pattern matching, governed by the exhaustiveness rule |
| `else` | `else` (a match pattern) | the catch-all match pattern |
| `record` | `(record (<field> <expr>)…)` | a **record** constructor — a value with a fixed, statically-known set of named fields, each field a possibly-distinct type |
| `list` | `(list <expr>…)` | a list literal |
| `map` | `(map (<key> <value>)…)` | a **map** literal — a dynamic, homogeneous key→value association (distinct from a record) |
| `.` | `(. <record> <key>)` | **member access** — the sole accessor into a record: projects the field/export `<key>` of the record `<record>` |
| `meta` | `(. <module> (meta <name>))` | a metadata key: reaches a module's non-export metadata (e.g. `capabilities`, `entry`) via `.`, kept out of the export namespace |
| `=` | `(= <a> <b>)` | structural-equality comparison |
| `+` `+%` | `(+ <a> <b>)` / `(+% <a> <b>)` | checked addition; wrapping addition (distinct wrapping type) |
| `unit` | `unit` | the unit value, the normal-termination value of an effect-only program (e.g. one whose `main` only emits events); its type is `Unit` |

**Member access is `(. <record> <key>)`.** `.` is the single accessor into a **record** — a value
with a fixed, statically-known set of named fields (a module's exports, a prelude namespace, a
`record` literal). `(. p x)` projects field `x` of record `p`; `(. Sign Neg)` projects the `Neg`
export of the prelude module-record `Sign`; `(. List at)` projects the `at` function of the `List`
prelude record. A key of the form `(meta <name>)` reaches a module's metadata channel rather than an
export — `(. m (meta capabilities))`, `(. m (meta entry))` — so metadata can never collide with an
export named, say, `capabilities`. `.` **never** accesses a **map**: a map is a dynamic, homogeneous
association whose lookup can fail, reached by a map operation (`(. Map at)` etc.) rather than by `.`,
precisely so that static member access resolves against a known field set and a future type-checker can
reject an unknown field. This record-versus-map distinction — fixed heterogeneous fields versus dynamic
homogeneous entries — is load-bearing for the type system.

The **dotted display form `a.b` is sugar**: a textual syntax renders `(. a b)` as `a.b` and a reader
expands `a.b` back to `(. a b)`, so a qualified name (`Sign.Neg`, `Int64.max`, `List.at`) and a field
projection (`p.x`) are the *same* construct — a member access node — with no lexical ambiguity in the
canonical tree, which carries only `(. …)`. There is no separate "qualified name" atom kind: `Sign`,
`Int64`, `List`, `Bytes`, `Option` are ordinary names bound (in the prelude) to record values, and the
dot looks a member up in them.

**Function application** is written `(<fn-expr> <arg>…)` where the head is an *expression that
evaluates to a function* (a name bound to a `fn`, a `def`, or an inline `(fn …)`), rather than a
`cadenza/core` construct symbol. This is how a program applies a first-class function value
(core-semantics.md §"A Function Is A First-Class Value", §"Applying A Function Binds Its Parameters To
Its Arguments"); a top-level `(def (<name> <param>…) <body>…)` is sugar for binding `<name>` to a
`(fn (<param>…) <body>…)`. A head that resolves to a core construct symbol names that construct; a head
that resolves to a bound function value applies it.

**Sequencing is explicit: `(do <form>…)`.** A single expression is a single value; when a scope needs to
evaluate several forms in order — typically to emit events for their effect before yielding a result —
it wraps them in a `do` block, which evaluates each form in order and yields the last form's value. This
replaces the `(let ((_ <effect>)) <rest>)` idiom of binding an effect to a throwaway name purely to
sequence it. A body that grammatically takes one form (`let`, `fn`, a `match` arm) admits a sequence by
holding a `(do …)`; the multi-form bodies of `def` and `module` are read as an implicit `do` over their
forms. Because a **declaration form binds its name in its enclosing scope** (a `module` binds its name to
its export record — core-semantics.md §"A Module Binds Its Name In Its Enclosing Scope"; a nested `def`
binds its name), the name a declaration introduces is in scope for the forms that follow it in the same
`do` block. So a program is naturally a `do` block of module declarations followed by a form that uses
them — `(do (module m …) ((. m main)))` — with no separate binding form wrapping each declaration.

Names and the numeric/collection/byte operations a program calls resolve as **member accesses into
prelude records** (via `.`, as sugar): `Sign.Neg` is `(. Sign Neg)`, `List.at` is `(. List at)`,
`Int64.max` is `(. Int64 max)`, `Bytes.of` is `(. Bytes of)`, and so on. `Sign`, `Option`, `List`,
`Int64`, `Float64`, `Rational`, `Wrapping64`, `Bytes`, `String`, `Bool`, `Unit` are ordinary names
bound in the prelude to **record** values whose fields are the variants/constructors/operations named
after the dot; `Some`/`None` are `Option`'s variant constructors. None of this is additional core
syntax — it is `.` (member access) applied to prelude records — and the corpus grounds each prelude
record where it is used. The floating-point not-a-number literal is written `nan` and denotes the canonical
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
