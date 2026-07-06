# Executable Semantics

This directory is the **single source of truth for what every Cadenza construct does**. It is
normative *by execution*, not by RFC-2119 extraction: it carries no MUST sentences and is not listed
in the duvet requirement gate. Its gate is the **behavior gate** — every case here must execute to
its recorded output on a promoted compiler (see
[capabilities/conformance-gate.md](../capabilities/conformance-gate.md) §"The Behavior Gate" and
[capabilities/compiler-pipeline.md](../capabilities/compiler-pipeline.md) §"The Behavior Gate").

This corpus exists because earlier Cadenza let the meaning of the language live in several places at
once — an interpreter, a separate document, a generated implementation, and a formal model — which
drifted apart (see [learnings/2026-07-02-parallel-semantics-drifted.md](../learnings/2026-07-02-parallel-semantics-drifted.md)).
There is now one place a construct's meaning lives, and it is runnable. This corpus itself is the
behavioral oracle: its recorded results are the authority the compiler and every tool agree with. A
reference interpreter (see
[capabilities/self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md)) is an
optional, independent realization that MAY cross-check those results, not the oracle.

## The form: s-expression cases

Each case is an **s-expression**, so the whole corpus is parseable by a minimal reader — the seed
toolchain needs only an s-expression reader to run the behavior gate, not the full surface parser.
This is deliberately the easiest thing to bootstrap. Cases live in `NN-feature.sexp` files, one
feature per file.

A case is a small fixed test-DSL vocabulary wrapping program fragments that are themselves written in
Cadenza's **canonical homoiconic representation** (see [`options/code-shape/`](../../options/code-shape/)):

```
(case "integer addition"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "no implicit promotion between numeric types"
  (input    (+ 2 2.0))
  (trap     "numeric type mismatch")
  (compiler (error CDZ0301)))

(case "a documented case"
  (doc    "Notes for humans and agents; part of the case, not stripped.")
  (input  (let ((x 10)) x))
  (output (: 10 Int64)))
```

### The test-DSL vocabulary

Each case has one `input`, one **primary result clause** (the recorded behavior of the one executable
semantics, which is the oracle), and optional annotations. The corpus is **one flat set**: differences
between generations are annotated *inline* rather than split into separate files, so there is exactly
one place a construct's meaning lives.

- `(case "<description>" <clause>...)` — one case; the description is a short human/agent-readable label.
- `(input <program>)` — the program to run, in the canonical representation.
- `(doc "<text>")` — optional prose attached to the case; documentation, never affecting the check.

**Primary result clause — exactly one, the oracle.** This is the recorded result the corpus fixes for
`input`; every generation that runs the case reproduces it, and the corpus's recorded value — not any
one implementation — is the authority (constitution §IX; §XIV). Usually a *terminal clause* — the
outcome of running the program:
- `(output <value-form>)` — the value the run produces on normal termination.
- `(trap "<reason>")` — the run halts at a defined point with this reason (for example, a checked overflow).
- `(exhausted)` — the run halts by exhausting the deterministic resource measure (the third terminal
  condition, distinct from a normal result and a trap — core-semantics.md §"A Program Terminates In
  Exactly One Terminal Condition").

For a program the compiler **rejects at compile time** rather than runs — whether the rejection needs
no type system (an unbound name, core-semantics.md §"Binding Is Lexical", or an undeclared capability,
capabilities-and-effects.md §"Undeclared Capability Is A Compile-Time Error") or is a type rejection (a
nominal-boundary comparison, a numeric mismatch, a non-exhaustive match, a contradicting annotation) —
the primary clause is instead:
- `(error <CODE>)` — the diagnostic code the rejection carries (from the pinned registry,
  [`options/diagnostics-schema/`](../../options/diagnostics-schema/)). A rejection is the program's
  recorded outcome: an ill-typed or ill-formed program has no run and therefore no terminal value, so
  the corpus records the rejection itself rather than what some evaluator might have produced had the
  program run. Cadenza has one implementation kind — a compiler — so there is no second, dynamic
  outcome to record.

**Observation clause — optional.**
- `(host-calls <call>...)` — the exact ordered sequence of host calls the run makes, each `<call>`
  written `(call <fn> <arg-value-form>...)` where `<fn>` names an imported host function; part of
  observable behavior (core-semantics.md §"Host Calls Are Ordered And Part Of Observable Behavior").
  `(host-calls)` asserts none was made. Which host functions exist is the target's concern, not the
  language's; a case names whatever WIT-typed host function it imports (see the `(import (host …))`
  form below).

**Host-response fixture — optional.**
- `(host-responses <respond>...)` — the response each host call returns, supplied in call order, each
  `<respond>` written `(respond <fn> <value-form>)`. Because a run's behavior is a deterministic
  function of its input **and the responses to its host calls**, a case whose program consumes a host
  call's return value fixes those responses here so the recorded result is reproducible. A host call
  whose WIT signature returns unit needs no `respond`. This fixture is exactly the replay log the host
  owns under the suspend-replay boundary (capabilities-and-effects.md §"Suspension Is Replay From The
  Host's Log"): every generation feeds the responses in order and reproduces the recorded terminal.

**Importing a host function.** A program that makes a host call declares the function it imports with a
complete WIT-typed signature, so the compiler can emit the import into the component's world
(host-interface-binding.md §"A Host Import Is A WIT-Typed Function The Manifest Enumerates"):
`(import (host <name> (func (<param-type>...) <result-type>)))` inside a module, declared alongside
`(use (capability <name>))`.

**Incremental realization.** A `(error <CODE>)` primary is the outcome every generation must
eventually produce, but the static-typing floor is realized **incrementally** over the type rules a
generation's compiler covers (constitution §VII; Amendment 0.4.0). For a rule it does not *yet* cover,
a generation MUST **decline** — refuse to derive a component — rather than run the ill-typed program to
a wrong value (reject-don't-miscompile,
[`spec/learnings/2026-07-03-decline-do-not-miscompile.md`](../learnings/2026-07-03-decline-do-not-miscompile.md)).
The differential gate treats a decline as *todo*, not as disagreement, so a green gate still means
every program a generation *does* compile agrees with the recorded outcome. There is no separate
"dynamic" outcome recorded alongside the rejection: the `(error <CODE>)` clause is the whole story.

**Generation-divergence annotation — optional, inline.**
- `(needs <capability>)` — the `input` requires a capability to be evaluated at all (e.g.
  `numeric-model` for rational/float arithmetic, `effects` for the algebraic-handler layer). A
  generation runs the case only if it realizes `<capability>` (conformance-gate.md §"A Generation Is
  Judged Against The Capabilities It Realizes"; `options/realized-capability-set/`). A case with no
  `(needs …)` is core — every generation, including the seed, runs it.

The result value form is `(: <value> <Type>)` — a value paired with its type — serialized under the
canonical value form ([`contracts/deterministic-value-form.md`](../contracts/deterministic-value-form.md)),
so a case's expected output is byte-exact. A case that carries neither `(compiler …)` nor `(needs …)`
is one every generation realizes and reproduces from the recorded oracle — the common case, and the
concrete meaning of "a well-typed program does not go wrong."

## Authoring rules

- **A case is executable.** Every case must be runnable — by a compiled component, and optionally by a
  reference interpreter — and carry a definite primary result clause — a terminal clause (`output`,
  `trap`, `exhausted`) or a front-end `error` (unbound name or undeclared capability) — optionally with
  a `host-calls` observation, a `host-responses` fixture, and inline `(compiler …)` / `(needs …)`
  annotations; a case with no definite primary result is not a case.
- **A case covers one behavior.** Prefer many small cases over one large program, so a behavior-gate
  failure names the construct that broke.
- **The corpus is complete per realized capability.** Every behavioral requirement of a capability a
  generation *realizes* is witnessed by at least one case that generation runs, so its behavior gate
  exercises what its requirement gate cites (conformance-gate.md §"A Generation Is Judged Against The
  Capabilities It Realizes").
- **Determinism is part of the check.** A case's output is byte-exact; a construct whose output could
  vary is either given a deterministic specification or is not admitted.

## Which cases a generation runs

A generation's behavior gate runs the cases whose required capabilities it **realizes**, not every
case ever authored (conformance-gate.md §"A Generation Is Judged Against The Capabilities It Realizes";
`options/realized-capability-set/`). This is a per-case filter, not a directory split:

- A case with **no** `(needs …)` is core — every generation runs it, including the seed.
- A case with `(needs <capability>)` runs only on a generation that realizes `<capability>`.
- A `(output …)` / `(trap …)` / `(exhausted)` primary clause is the recorded result every running
  generation must reproduce when it runs the program.
- An `(error <CODE>)` primary clause is the rejection every generation that covers the relevant rule
  must produce; a generation that does not yet cover the rule **declines** (reject-don't-miscompile)
  and the gate scores that as todo, not disagreement. So mixed-type arithmetic `(+ 2 2.0)` records the
  single outcome `(error CDZ0301)` — the rejection is the behavior, not a footnote to a value the
  program never produces.

The **seed** is a compiler that realizes the static-typing floor incrementally
(constitution §VII; Amendment 0.4.0;
`../learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md`). It thus runs every
`(needs …)`-free case: producing the `(error <CODE>)` rejection where a case records one for a rule it
covers, reproducing the terminal clause otherwise, and enforcing the capability floor. It
realizes lowering, binding, control flow, matching, structural equality, the static-typing floor,
runtime traps that survive type-checking (overflow, division by zero, index out of bounds,
exhaustion), observable behavior, and the primitive value forms — and nothing that a `(needs …)`
marks as a later generation's, nor a type rule it does not yet cover (which it declines rather than
miscompiles).

## Files

The corpus is organized by feature, numbered for a natural reading order — one flat set; generation
differences are inline annotations, not separate files. It grows as capabilities are specified.

- `01-literals.sexp` — literals and their types
- `02-binding-and-control.sexp` — lexical binding, shadowing, `do` sequencing (in-order evaluation, last-form value), conditionals, pattern bindings, unbound-name rejection
- `03-equality-and-observation.sexp` — structural/float equality, ordering, ordered host calls, resource-measure exhaustion
- `04-capabilities.sexp` — the mandatory capability-declaration floor: a program imports WIT-typed host functions, reaching an undeclared one is rejected, an empty manifest is pure, and a response-returning host call fixes its response with `(host-responses …)`
- `05-compound-types.sexp` — records, sum types, lists, maps; member access (the `.` accessor: field read, non-record trap, missing-field trap); structural equality (runtime) with `(compiler …)` for the static nominal/structural and exhaustiveness rejections
- `06-numeric-model.sexp` — checked `Int64` core; `(compiler …)` for compile-time no-promotion; `(needs numeric-model)` for rational/wrapping/floating-point arithmetic
- `07-type-system.sexp` — annotation-vs-inference and ill-typedness, as `(compiler …)` rejections the typed compiler makes; the primary clause records what an untyped dynamic evaluator would produce
- `08-self-hosting-surface.sexp` — reader/printer round-trip over a program's AST, as `(needs self-hosting-surface)` cases a later generation realizes (no `eval`: the compiler needs AST construction, not execution)
- `09-functions.sexp` — first-class functions and closures: `fn` values, application, closure capture, higher-order functions, recursion, and resource-measure exhaustion on unbounded recursion (core; the seed realizes these)
- `10-bytes.sexp` — the `Bytes` byte-sequence value form (construction, equality, length, concatenation, out-of-range construction trap), tagged `(needs bytes)`; the seed realizes it so the Cadenza-authored compiler can build a component's wasm bytes as an ordinary value (bootstrap.md §"The Self-Hosted Compiler Is Authored In Cadenza"; `options/realized-capability-set/`). Its indexing and slicing are fallible (Option-returning), tagged `(needs fallible-access)` — a capability the seed does not yet realize (collections-and-text.md §"Indexing And Lookup Are Fallible, Not Trapping")
- `11-modules.sexp` — single-module semantics: a module declaration binds its name in the enclosing scope (used via a `do` block, no `let` wrapping) to a record of its exports (each `def` a reachable export field), and carries its capability manifest and entry as metadata reached by a `(meta …)` key distinct from every export, so a declared capability is not an export and a like-named export and metadata key do not collide (core-semantics.md §Modules); multi-module composition (imports, visibility, cycles) is deferred beyond a single module (`options/realized-capability-set/`)
- `14-effects-and-handlers.sexp` — suspend-and-replay across a host call (the host owns the response log; a run holds no resume state), an intra-program effect discharged by a handler that does not escape to the manifest, one-shot continuations, and purity as the empty row; `(needs effects)` cases a later generation realizes
- `15-rows-and-open-sums.sexp` — row-polymorphic open records (a function open over a record's extra fields; subset comparison as explicit projection), open sums with a mandatory open-tail arm, and schema-typed payload decoding to a typed result; `(needs rows)` / `(needs open-sums)` cases a later generation realizes
- `16-binary-matching.sexp` — the `(bin …)` binary construction-and-matching form (one dual keyword: construct in expression position, destructure in pattern position) over the `Bytes` value form: fixed-width integer segments with explicit endianness/signedness, sub-byte bit-fields, and dependent-size `bytes` segments; static byte-alignment rejection (`CDZ0220`) and runtime segment-fit trap; `(needs binary-matching)` cases a later generation realizes (`options/binary-syntax/`)
- `17-symbols.sexp` — the `Symbol` interned-name value form: `Symbol.of` interns a `String` to a `Symbol` compared by content in constant time, `Symbol.to-string` recovers it, with a `#"<text>"` reader literal; a Symbol is a nominal value over `String`, so equality reuses String equality and the nominal boundary reuses `CDZ0202`; `(needs symbols)` cases a later generation realizes (`options/symbol-interning/`)
- `18-units-of-measure.sexp` — the optional, compile-time-only dimensional-analysis layer over the numeric core: a quantity `(Qty T u)` pairs an underlying numeric type `T` with a compile-time unit `u` drawn from a free abelian group over `Symbol`-named base dimensions (`Unit.one`/`Unit.base`/`Unit.*`/`Unit./`/`Unit.^`); each dimension is a family of interconvertible units carrying an exact `Rational` scale to a reference unit, so units of one dimension mix (`1 inch + 1 mm`) by automatic exact conversion (`Unit.of`/`Unit.in`) while a mix of dimensions stays `CDZ0501`, with SI-decimal and IEC-binary prefixes as exact scales (`Unit.prefix`); `+`/`-`/comparison require equal dimension, `*`/`/` compose it, a mismatch is the compile-time `CDZ0501`, and `(Qty T u)` erases its dimension to `T` (a scale conversion is the exact arithmetic the source denotes); `(needs units-of-measure)` cases a later generation realizes (`options/units-of-measure/`)

Planned as the capabilities they witness are filled in: traits as explicitly-passed dictionaries,
documentation and comments (each a node the compiler sees through, witnessing that it is semantically
inert), verification.
