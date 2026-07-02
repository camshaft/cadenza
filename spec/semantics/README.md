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
There is now one place a construct's meaning lives, and it is runnable. The reference interpreter
(see [capabilities/self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md)) is
the realization of this corpus and the behavioral oracle; the compiler and every tool agree with it.

## The form: s-expression cases

Each case is an **s-expression**, so the whole corpus is parseable by a minimal reader — the seed
toolchain needs only an s-expression reader plus the reference interpreter to run the behavior gate,
not the full surface parser. This is deliberately the easiest thing to bootstrap. Cases live in
`NN-feature.sexp` files, one feature per file.

A case is a small fixed test-DSL vocabulary wrapping program fragments that are themselves written in
Cadenza's **canonical homoiconic representation** (see [`options/code-shape/`](../../options/code-shape/)):

```
(case "integer addition"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "no implicit promotion between numeric types"
  (input  (+ 2 2.0))
  (error  CDZ0301))

(case "a documented case"
  (doc    "Notes for humans and agents; part of the case, not stripped.")
  (input  (let ((x 10)) x))
  (output (: 10 Int64)))
```

### The test-DSL vocabulary

Each case has one `input`, one **interpreter terminal clause** (the primary result — the behavior of
the one executable semantics, which is the oracle), and optional annotations. The corpus is **one flat
set**: differences between generations are annotated *inline* rather than split into separate files, so
there is exactly one place a construct's meaning lives.

- `(case "<description>" <clause>...)` — one case; the description is a short human/agent-readable label.
- `(input <program>)` — the program to run, in the canonical representation.
- `(doc "<text>")` — optional prose attached to the case; documentation, never affecting the check.

**Primary result clause — exactly one, the oracle.** This is what the reference interpreter produces
for `input`; every generation reproduces it for the cases it runs. Usually a *terminal clause* — the
outcome of running the program:
- `(output <value-form>)` — the value the run produces on normal termination.
- `(trap "<reason>")` — the run halts at a defined point with this reason (for example, a checked overflow).
- `(exhausted)` — the run halts by exhausting the deterministic resource measure (the third terminal
  condition, distinct from a normal result and a trap — core-semantics.md §"A Program Terminates In
  Exactly One Terminal Condition").

For a program the interpreter's own front-end refuses **before** running it — a rejection that needs
no type system, namely an unbound name (core-semantics.md §"Binding Is Lexical") or an undeclared
capability (capabilities-and-effects.md §"Undeclared Capability Is A Compile-Time Error", the
mandatory floor) — the primary clause is instead:
- `(error <CODE>)` — the diagnostic code the compile-time rejection carries (from the pinned registry,
  [`options/diagnostics-schema/`](../../options/diagnostics-schema/)). This is a rejection *every*
  generation makes, including the dynamic seed, because scope resolution and the capability floor are
  intrinsic to evaluation and do not require static typing — distinct from the `(compiler …)`
  annotation below, which only a *typed* generation checks.

**Observation clause — optional.**
- `(events <event>...)` — the exact ordered sequence of events the run emits, each `<event>` written
  `(event <kind> <payload-value-form>)`; part of observable behavior (core-semantics.md §"Emitted
  Events Are Ordered And Part Of Observable Behavior"). `(events)` asserts none was emitted.

**Generation-divergence annotations — optional, inline.**
- `(compiler (error <CODE>))` — a generation with a static front-end **rejects** this `input` at
  compile time with this diagnostic code (from the pinned registry,
  [`options/diagnostics-schema/`](../../options/diagnostics-schema/)) *instead of* running it to the
  interpreter terminal clause. A generation that realizes static typing checks this clause; a dynamic
  generation (the seed) ignores it and checks the interpreter clause. The compiler may diverge **only
  by rejecting** — if it runs a program at all, it MUST agree with the interpreter (constitution §XIV
  oracle agreement), so there is no `(compiler (output …))`.
- `(needs <capability>)` — the `input` requires a capability to be evaluated at all (e.g.
  `numeric-model` for rational/float arithmetic). A generation runs the case only if it realizes
  `<capability>` (conformance-gate.md §"A Generation Is Judged Against The Capabilities It Realizes";
  `options/realized-capability-set/`). A case with no `(needs …)` is core — every generation, including
  the seed, runs it.

The result value form is `(: <value> <Type>)` — a value paired with its type — serialized under the
canonical value form ([`contracts/deterministic-value-form.md`](../contracts/deterministic-value-form.md)),
so a case's expected output is byte-exact. A case that carries neither `(compiler …)` nor `(needs …)`
is one where the compiler and interpreter agree and every generation realizes it — the common case,
and the concrete meaning of "a well-typed program does not go wrong."

## Authoring rules

- **A case is executable.** Every case must be runnable by the reference interpreter and carry a
  definite primary result clause — a terminal clause (`output`, `trap`, `exhausted`) or a front-end
  `error` (unbound name or undeclared capability) — optionally with an `events` observation and inline
  `(compiler …)` / `(needs …)` annotations; a case with no definite primary result is not a case.
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
`options/realized-capability-set/`). Because divergence is annotated inline, this is a per-case filter,
not a directory split:

- A case with **no** `(needs …)` is core — every generation runs it, including the seed.
- A case with `(needs <capability>)` runs only on a generation that realizes `<capability>`.
- The **interpreter** terminal clause (`output`/`trap`/`exhausted`) is the oracle every running
  generation must reproduce.
- A `(compiler (error …))` annotation is checked **only** by a generation that realizes static
  typing; a dynamic generation (the seed — constitution §VII bootstrap carve-out;
  `../learnings/2026-07-02-seed-is-a-dynamic-interpreter.md`) ignores it and checks the interpreter
  clause. So the same case, e.g. mixed-type arithmetic `(+ 2 2.0)`, records **both** the seed's runtime
  `(trap "numeric type mismatch")` and the typed generation's `(compiler (error CDZ0301))` — in one
  place, not two files.

The **seed** thus runs: every `(needs …)`-free case, checking interpreter clauses and the capability
floor, and ignoring `(compiler …)` annotations. It is a dynamic tree-walking interpreter, so it
realizes evaluation, binding, control flow, runtime matching, structural equality, traps, observable
behavior, the mandatory capability floor, and the primitive value forms — and nothing that a
`(needs …)` or `(compiler …)` marks as a later generation's.

## Files

The corpus is organized by feature, numbered for a natural reading order — one flat set; generation
differences are inline annotations, not separate files. It grows as capabilities are specified.

- `01-literals.sexp` — literals and their types
- `02-binding-and-control.sexp` — lexical binding, shadowing, conditionals, pattern bindings, unbound-name rejection
- `03-equality-and-observation.sexp` — structural/float equality, ordering, emitted events, resource-measure exhaustion
- `04-capabilities.sexp` — the mandatory capability-declaration floor and undeclared-capability rejection
- `05-compound-types.sexp` — records, sum types, lists, maps; structural equality (runtime) with `(compiler …)` for the static nominal/structural and exhaustiveness rejections
- `06-numeric-model.sexp` — checked `Int64` core; `(compiler …)` for compile-time no-promotion; `(needs numeric-model)` for rational/wrapping/floating-point arithmetic
- `07-type-system.sexp` — annotation-vs-inference and ill-typedness, as `(compiler …)` divergences over inputs the dynamic interpreter still runs

Planned as the capabilities they witness are filled in: functions and closures, documentation,
verification.
