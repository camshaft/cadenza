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

- `(case "<description>" <clause>...)` — one case; the description is a short human/agent-readable label.
- `(input <program>)` — the program to run, in the canonical representation.
- `(output <value-form>)` — the exact result its execution must produce.
- `(error <CODE>)` — for a program that must be rejected at compile time, the expected diagnostic code
  ([`options/diagnostics-schema/`](../../options/diagnostics-schema/)).
- `(trap "<reason>")` — for a program that must halt at runtime (for example, a checked overflow).
- `(doc "<text>")` — optional prose attached to the case; documentation, never affecting the check.

The result value form is `(: <value> <Type>)` — a value paired with its type — serialized under the
canonical value form ([`contracts/deterministic-value-form.md`](../contracts/deterministic-value-form.md)),
so a case's expected output is byte-exact.

## Authoring rules

- **A case is executable.** Every case must be runnable by the reference interpreter and produce a
  definite `output`, `error`, or `trap`; a case with no definite result is not a case.
- **A case covers one behavior.** Prefer many small cases over one large program, so a behavior-gate
  failure names the construct that broke.
- **The corpus is complete per capability.** Every behavioral requirement in a capability spec is
  witnessed by at least one case here, so the behavior gate exercises what the requirement gate cites.
- **Determinism is part of the check.** A case's output is byte-exact; a construct whose output could
  vary is either given a deterministic specification or is not admitted.

## Files

The corpus is organized by feature, numbered for a natural reading order. It grows as capabilities are
specified.

- `01-literals.sexp` — literals and their types
- `06-numeric-model.sexp` — exactness, conversions, overflow, no implicit promotion

Planned as the capabilities they witness are filled in: binding and scope, functions and closures,
pattern matching, compound types, capabilities, documentation, verification.
