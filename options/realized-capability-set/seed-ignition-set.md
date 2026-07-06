# Realized Capability Set — Choice: seed-ignition-set

> **The default choice for the `realized-capability-set` decision** (see [README.md](./README.md)). It
> pins the capabilities the operator-synthesized **seed** generation realizes, against which its
> behavioral-witnessing obligation is judged (conformance-gate.md §"A Generation Is Judged Against The
> Capabilities It Realizes").

## The principle: a bootstrap realizes what a compiler needs, then iterates

The seed exists to clear the ignition bar — derive one real Cadenza program to a content-addressed
component, run it, and reproduce the recorded result in the conformance corpus (the oracle) — and to
derive the *next* generation. It
does **not** need every language feature to do that. A compiler manipulates abstract syntax trees
(records, sum types, lists, strings, symbols), evaluates a core (binding, scope, conditionals, pattern
matching, functions), type-checks, declares and binds capabilities, and derives a component. Features
that are not on that path are realized by **later Cadenza-authored generations**, which is the whole
point of the flywheel: get a minimal seed running, then iterate, rather than building features that are
not immediately useful for a compiler.

## Realized by the seed

The seed is a **native reference compiler** (`cdz-rustc`) that lowers Cadenza source to a component
and runs it (constitution Amendment 0.3.0;
`spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md`). It realizes exactly
the capabilities its compiler must lower and type-check to reproduce the seed corpus and clear the
ignition bar:

- **core-semantics** — evaluation, lexical binding, scope, shadowing, **first-class functions and
  closures** (function values that capture their scope, application, higher-order use, recursion
  bounded by the resource measure — core-semantics.md §Functions), conditionals, **`do` sequencing**
  (in-order evaluation, last-form value, and a declaration form binding its name for the forms that
  follow it — core-semantics.md §Sequencing, §"A Module Binds Its Name In Its Enclosing Scope"), runtime
  pattern matching, traps, terminal conditions, structural equality, ordering, and the
  observable-behavior projection (including emitted events and resource-measure exhaustion). Functions
  are realized by the seed because the first Cadenza artifact is a **compiler**, which is not expressible
  without them.
- **capabilities-and-effects** — the **mandatory capability-declaration floor** only (declare, reach,
  reject-undeclared at compile time, manifest-union). This is one of the static rejections the seed
  performs — alongside the static-typing floor below — and it is what makes a derived component
  *safe*: capability-safety is a never-downgradable Governance Floor. The **optional effect-tracking
  layer** is NOT realized.
- **type-system** (the static-typing floor) — realized incrementally over the type rules the seed's
  compiler covers, reject-don't-miscompile (constitution §VII;
  `spec/learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md`). The seed performs, as
  **compile-time** rejections, CDZ0301 (no implicit numeric promotion), CDZ0210 (match
  exhaustiveness), CDZ0202 (nominal/structural mismatch), CDZ0203 (annotation conflict), and CDZ0201
  (general ill-typedness); in the corpus these are the `(compiler (error …))` clauses the seed
  produces. Full Hindley-Milner inference and generics/monomorphization MAY be a later increment; the
  static-typing floor itself is not deferred.
- **compiler-pipeline**, **conformance-gate**, **self-hosting-and-bootstrap** — the pipeline phases,
  the two gates, the oracle, and the derivation modes.
- **primitive value forms the AST and the corpus need:** `Int64` (checked, trapping on overflow at
  runtime), `Bool`, `String`, `Float64` (literal, canonical byte form, and equality — including −0.0
  distinctness and canonical NaN), and `record` and sum-type values. These are the value forms the
  compiler represents an AST and a small program with. The seed **statically types** these values and
  **rejects an ill-typed program at compile time** with the type rule's diagnostic code (constitution
  §VII), realized incrementally over the rules it covers (reject-don't-miscompile).
- **`collections` (primitive slice):** the built-in `list` and `map` aggregates as value forms —
  construction and structural equality — which a compiler needs to build and
  walk an AST. Corpus cases needing them carry `(needs collections)`. Fallible indexing and lookup
  (Option-returning `at`/`get`; collections-and-text.md §"Indexing And Lookup Are Fallible, Not
  Trapping") are a distinct capability, `fallible-access`, the seed does not yet realize. This is the
  primitive slice only;
  the richer **`collections-and-text`** capability (string scalar semantics, string lexicographic
  ordering, map iteration-order determinism) is deferred (below).
- **`Bytes` (a byte-sequence value form):** an immutable sequence of 8-bit bytes — construction from a
  list of `Int64` in `0..=255` (values outside the range trap), concatenation, length,
  and structural equality — realized by the seed **because the Cadenza-authored compiler
  emits a component's wasm bytes as an ordinary `Bytes` value** (bootstrap.md §"The Compiler Is
  Authored In Cadenza, Not In The Seed"; self-hosting-and-bootstrap.md §"Each Generation Is Derived By
  The Previous"). This is the seam decision: the seed contributes evaluation and a `Bytes` value the
  compiler builds up; it does **not** contain any translation of a program's AST to component bytes —
  that translation is authored in Cadenza. `Bytes` is the one primitive added to the seed for this
  purpose; the wasm-binary and component-wrapping *encoders* are Cadenza source the seed runs, not seed
  code. (Earlier this build deferred all byte primitives; that deferral is lifted for `Bytes`
  specifically, because without it a Cadenza program cannot construct the component bytes the toolchain
  must generate — resolving the seed↔compiler seam ambiguity surfaced during the attended build.)

## Included in the language but NOT realized by the seed (deferred to later generations)

Each is included in the language by its declared default, but the seed does not realize it, so its
behavioral requirements are **not load-bearing for the seed's behavior gate** (conformance-gate.md
§"A Generation Is Judged Against The Capabilities It Realizes"). They re-enter as a later generation
realizes them and adds their witnessing cases:

- **type-system beyond the static-typing floor** — full Hindley-Milner inference (type variables over
  the whole structural/nominal universe) and generics/monomorphization. The seed realizes the
  static-typing floor (above, under "Realized by the seed"); the richer inference and generics land in
  a later increment.
- **numeric-model beyond the primitive core** — the **width-indexed integer types** beyond the default
  `Int64` (the constructors `(Int N)` / `(UInt N)` over a compile-time width `N` in `1..=64`, of which
  `Int8/16/32/64` and `UInt8/16/32/64` are the aliased widths — options/numeric-model/), the explicit
  checked (`T.of`) and truncating (`T.wrap`) integer conversions, exact **rational** arithmetic,
  arbitrary-precision **big-integers**, **wrapping** integer types, and deterministic floating-point
  **arithmetic** (rounding/FMA). The seed realizes only `Int64` (checked, trapping on overflow — every
  integer it lowers is 64-bit) and `Float64` literals and equality; it does not realize the other integer
  widths, integer conversions, rational, bignum, wrapping, or float arithmetic. The width-indexed integer
  types are the numeric-model increment scheduled for M4 (roadmap-to-self-hosting): a compiler needs
  `UInt8` for a module's bytes and `UInt32` for its section sizes and indices, so it is the highest-value
  numeric increment on the self-hosting path, but it is not on the *ignition* path (the seed clears
  ignition with `Int64` and `Bytes`), so it is realized by a later generation. Realizing it depends on
  the type-system increment that admits a **compile-time value (a width) as a type-constructor argument**
  and evaluates its `1..=64` constraint (`CDZ0302`), which rides on the same generics/monomorphization
  work; the seed today has no width-indexed types. Its corpus witnesses carry `(needs numeric-model)` and
  are skipped by the seed until then. Widths above 64 stay reserved to the big-integer layer and are not
  part of this increment.
- **collections-and-text beyond the primitive collections slice** — string Unicode-scalar semantics
  and length, string lexicographic ordering, string NFC equality, and map iteration-order determinism.
  The seed realizes the primitive `list`/`map` slice (above) for building an AST; the full capability's
  text and ordering semantics are realized later.
- **fallible-access** — Option-returning element indexing and key lookup: `List.at` / `String.at` /
  `Bytes.at` yield `(Option T)` (`Some` in bounds, `None` out of bounds), sub-sequence `slice` yields
  `(Option Seq)`, and map `get` yields `(Option V)` — with `expect` (core-semantics.md §"Requiring The
  Value Of An Optional Traps On Absence") the explicit combinator that turns a `None` into a trap
  carrying its message (collections-and-text.md §"Indexing And Lookup Are Fallible, Not Trapping"). The
  seed today traps directly on an out-of-bounds access rather than returning an Option, so it does not
  realize this capability; its corpus witnesses carry `(needs fallible-access)` and are skipped by the
  seed until a later generation returns the Option. This is a behavior change from the earlier
  total-or-trap indexing, not merely an addition.
- **binary-matching** — the `(bin …)` binary construction-and-matching form (options/binary-syntax/):
  fixed-width integer segments with explicit endianness and signedness, sub-byte bit-fields, and
  dependent-size `bytes` segments, in both expression (construct) and pattern (destructure) position.
  The seed realizes the primitive `Bytes` slice (construction, equality, length, concatenation)
  it needs to build a component's wasm bytes; the richer `bin` grammar that
  subsumes it lands in a later increment. Corpus cases carry `(needs binary-matching)`.
- **symbols** — the `Symbol` interned-name value form (options/symbol-interning/): `Symbol.of` interns
  a `String` to a `Symbol`, `Symbol.to-string` recovers it, and `=` compares two Symbols by content in
  constant time, with a `#"<text>"` reader literal. A Symbol is a nominal value over `String`, so its
  equality reuses String equality and its nominal boundary reuses `CDZ0202` (no new code, no new trap).
  A self-hosting compiler keys its symbol table on Symbols so a name comparison is a handle compare
  rather than an O(N) byte scan; it is the highest-leverage representation win on the self-hosting path
  but is not on the *ignition* path (the seed clears ignition with `Int64`, `Bytes`, and `String`), so
  it is realized by a later generation. Corpus cases carry `(needs symbols)`.
- **effect-tracking** (the optional layer of capabilities-and-effects), **verification-layers**,
  **property-based-testing**, **units-of-measure** — optional capabilities, included by default,
  realized later.
- **module-pragmas** — the `(pragma <key> …)` module-directive channel (modules-and-namespaces.md
  §"Module Directives"; options/module-pragmas/), whose one registered key today is `default-integer`.
  The seed does not parse pragmas, so the general-mechanism cases (unrecognized key `CDZ0601`, malformed
  args `CDZ0602`) carry `(needs module-pragmas)` and the `default-integer` behavior cases carry
  `(needs numeric-model)` (they also need the full numeric family the key names, e.g. `BigInt`); a later
  generation realizes both. Note the seed still lowers a pragma-free module unchanged — the default
  literal type stays `Int64` when no pragma is present, which is exactly the no-`(needs …)` module cases.
- **self-hosting-surface** — the Cadenza-authored reader/printer round-trip and the *optional*
  reference interpreter authored *in Cadenza* (self-hosting-surface.md). The seed is the
  foreign-language **compiler** and provides the reader/printer natively; a Cadenza-authored
  reader/printer and reference interpreter are an optional (`MAY`) later oracle
  (`spec/capabilities/self-hosting-and-bootstrap.md` §"The Seed Compiler Is The One Step Outside The
  Loop"), not a bootstrap rung. Corpus cases carry `(needs self-hosting-surface)`.
- **metaprogramming** (macros), **modules-and-namespaces** beyond a single module, the
  **memory-and-resource-model** surface, the full **diagnostics** tooling beyond coded rejections,
  **tooling-and-lsp**, and **agent-authoring** beyond direct binary-AST read/construct.

## How this scopes the seed's gates

- **Behavior gate.** The seed's behavior gate runs the corpus cases whose required capabilities it
  realizes (conformance-gate.md §"The Corpus Is A Gate" as scoped by §"A Generation Is Judged Against
  The Capabilities It Realizes"). Concretely the seed runs every case with no `(needs …)` annotation,
  checks each case's recorded result (the corpus is the oracle), and produces the
  `(compiler (error …))` rejection where a case records one (constitution §VII). Cases tagged
  `(needs numeric-model)` (rational, bignum, wrapping, float arithmetic) are
  not run by the seed; a generation that realizes the full numeric-model runs them. Nothing lives in a
  separate directory — the corpus is one flat set annotated inline (spec/semantics/README.md).
- **Requirement gate.** Independently scoped by `.duvet/bootstrap.toml` (the ignition requirement
  subset). The two subsets are consistent: the seed realizes the ignition-subset capabilities plus the
  primitive value forms the corpus exercises.

Recorded in `implementation/DECISIONS.md` for the current build so the realized set the seed was judged
against is reproducible.
