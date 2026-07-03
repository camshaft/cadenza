# Realized Capability Set — Choice: seed-ignition-set

> **The default choice for the `realized-capability-set` decision** (see [README.md](./README.md)). It
> pins the capabilities the operator-synthesized **seed** generation realizes, against which its
> behavioral-witnessing obligation is judged (conformance-gate.md §"A Generation Is Judged Against The
> Capabilities It Realizes").

## The principle: a bootstrap realizes what a compiler needs, then iterates

The seed exists to clear the ignition bar — derive one real Cadenza program to a content-addressed
component, run it, and agree with the reference interpreter — and to derive the *next* generation. It
does **not** need every language feature to do that. A compiler manipulates abstract syntax trees
(records, sum types, lists, strings, symbols), evaluates a core (binding, scope, conditionals, pattern
matching, functions), type-checks, declares and binds capabilities, and derives a component. Features
that are not on that path are realized by **later Cadenza-authored generations**, which is the whole
point of the flywheel: get a minimal seed running, then iterate, rather than building features that are
not immediately useful for a compiler.

## Realized by the seed

The seed is a **dynamic tree-walking interpreter** (constitution §VII bootstrap carve-out;
`spec/learnings/2026-07-02-seed-is-a-dynamic-interpreter.md`). It realizes exactly the capabilities its
reference interpreter must evaluate to reproduce the seed corpus and clear the ignition bar:

- **core-semantics** — evaluation, lexical binding, scope, shadowing, conditionals, runtime pattern
  matching, traps, terminal conditions, structural equality, ordering, and the observable-behavior
  projection (including emitted events and resource-measure exhaustion).
- **capabilities-and-effects** — the **mandatory capability-declaration floor** only (declare, reach,
  reject-undeclared at compile time, manifest-union). This is the one static rejection the seed
  performs, because it is what makes a derived component *safe* — capability-safety is a
  never-downgradable Governance Floor. The **optional effect-tracking layer** is NOT realized.
- **compiler-pipeline**, **conformance-gate**, **self-hosting-and-bootstrap** — the pipeline phases,
  the two gates, the oracle, and the derivation modes.
- **primitive value forms the AST and the corpus need:** `Int64` (checked, trapping on overflow at
  runtime), `Bool`, `String`, `Float64` (literal, canonical byte form, and equality — including −0.0
  distinctness and canonical NaN), and `record` and sum-type values. These are the value forms a
  dynamic interpreter represents an AST and a small program with. The seed manipulates these values
  **without static types**; it evaluates and, where an operation has no defined result, traps at
  runtime rather than rejecting at compile time.
- **`collections` (primitive slice):** the built-in `list` and `map` aggregates as value forms —
  construction, structural equality, and total-or-trap indexing — which a compiler needs to build and
  walk an AST. Corpus cases needing them carry `(needs collections)`. This is the primitive slice only;
  the richer **`collections-and-text`** capability (string scalar semantics, string lexicographic
  ordering, map iteration-order determinism) is deferred (below).

## Included in the language but NOT realized by the seed (deferred to later generations)

Each is included in the language by its declared default, but the seed does not realize it, so its
behavioral requirements are **not load-bearing for the seed's behavior gate** (conformance-gate.md
§"A Generation Is Judged Against The Capabilities It Realizes"). They re-enter as a later generation
realizes them and adds their witnessing cases:

- **type-system** — static typing, inference, annotation checking, and generics/monomorphization
  (constitution §VII bootstrap carve-out). The seed is dynamic: it does no static type-checking. The
  first generation derived after the seed realizes the static-typing floor. Consequently the
  compile-time rejections a typed compiler makes — no implicit numeric promotion (CDZ0301), match
  exhaustiveness (CDZ0210), nominal/structural mismatch (CDZ0202), annotation conflict (CDZ0203),
  general ill-typedness (CDZ0201) — are NOT performed by the seed; where such a program has a defined
  dynamic outcome the seed evaluates or traps. In the corpus each such case records the interpreter's
  dynamic result as its primary clause and the typed generation's rejection as an inline
  `(compiler (error …))` annotation that the seed ignores (spec/semantics/README.md §"Which cases a
  generation runs").
- **numeric-model beyond the primitive core** — exact **rational** arithmetic, arbitrary-precision
  **big-integers**, **wrapping** integer types, and deterministic floating-point **arithmetic**
  (rounding/FMA). The seed realizes `Int64` (checked, trapping on overflow) and `Float64` literals and
  equality; it does not realize rational/bignum/wrapping/float arithmetic.
- **collections-and-text beyond the primitive collections slice** — string Unicode-scalar semantics
  and length, string lexicographic ordering, string NFC equality, and map iteration-order determinism.
  The seed realizes the primitive `list`/`map` slice (above) for building an AST; the full capability's
  text and ordering semantics are realized later.
- **effect-tracking** (the optional layer of capabilities-and-effects), **verification-layers**,
  **property-based-testing**, **units-of-measure** — optional capabilities, included by default,
  realized later.
- **bootstrap-interpreter** — the reader/printer and the interpreter authored *in Cadenza*
  (bootstrap-interpreter.md). The seed is the foreign-language interpreter; the Cadenza-authored one
  is a later rung (`spec/capabilities/self-hosting-and-bootstrap.md` §"The Interpreter Is Proven As A
  Component Before It Is Iterated On"). Corpus cases carry `(needs bootstrap-interpreter)`.
- **metaprogramming** (macros), **modules-and-namespaces** beyond a single module, the
  **memory-and-resource-model** surface, the full **diagnostics** tooling beyond coded rejections,
  **tooling-and-lsp**, and **agent-authoring** beyond direct binary-AST read/construct.

## How this scopes the seed's gates

- **Behavior gate.** The seed's behavior gate runs the corpus cases whose required capabilities it
  realizes (conformance-gate.md §"The Corpus Is A Gate" as scoped by §"A Generation Is Judged Against
  The Capabilities It Realizes"). Concretely the seed runs every case with no `(needs …)` annotation,
  checks each case's interpreter primary clause (the oracle), and ignores every `(compiler …)`
  annotation. Cases tagged `(needs numeric-model)` (rational, bignum, wrapping, float arithmetic) are
  not run by the seed; a generation that realizes the full numeric-model runs them. Nothing lives in a
  separate directory — the corpus is one flat set annotated inline (spec/semantics/README.md).
- **Requirement gate.** Independently scoped by `.duvet/bootstrap.toml` (the ignition requirement
  subset). The two subsets are consistent: the seed realizes the ignition-subset capabilities plus the
  primitive value forms the corpus exercises.

Recorded in `implementation/DECISIONS.md` for the current build so the realized set the seed was judged
against is reproducible.
