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

## Case format

Each feature is one markdown file. Each case is an **Input** paired with the **Output** its execution
must produce:

````markdown
### Case: a short description

**Input:**

```cadenza
<a program in the canonical display>
```

**Output:**

```
<the exact output its execution produces>
```
````

The Input is written in the **canonical display** of the homoiconic representation (see
[`defaults/code-shape.md`](../../defaults/code-shape.md)). Because display is decoupled from
representation, a case's meaning is a property of the representation the display denotes, not of the
display's surface; a build that offers an alternative display renders the same case identically after
projecting through the representation. A case's Output is the observable result the reference
interpreter produces, serialized under the canonical value form
([`contracts/deterministic-value-form.md`](../contracts/deterministic-value-form.md)).

## Authoring rules

- **A case is executable.** Every case must be runnable by the reference interpreter and produce a
  definite output; a case with no definite output is not a case.
- **A case covers one behavior.** Prefer many small cases over one large program, so a behavior-gate
  failure names the construct that broke.
- **The corpus is complete per capability.** Every behavioral requirement in a capability spec is
  witnessed by at least one case here, so that the behavior gate exercises what the requirement gate
  cites.
- **Determinism is part of the check.** A case's output is byte-exact; a construct whose output could
  vary is either given a deterministic specification or is not admitted.

## Files

The corpus is organized by feature, numbered for a natural reading order. It is migrated and
re-derived from the executable specification of earlier Cadenza generations against the canonical
representation this specification defines; the numbering below grows as capabilities are specified.

- `01-literals.md` — literals and their types
- `02-binding-and-scope.md` — binding, lexical scope, shadowing
- `03-functions.md` — definition, application, closures
- `04-pattern-matching.md` — matching, exhaustiveness, bindings
- `05-compound-types.md` — records, tuples, sums, lists
- `06-numeric-model.md` — exactness, conversions, overflow, no implicit promotion
- `07-capabilities.md` — capability declaration and rejection of undeclared reach
- `08-verification.md` — contracts and refinements as an opt-in layer
