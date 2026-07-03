# Diagnostics Schema — Choice: coded-span-record

> **The default choice for the `diagnostics-schema` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete machine-readable
> diagnostic record that realizes the diagnostics.md capability's requirements that every diagnostic
> carry a stable code, a precise span, and the rule it enforces.
>
> Diagnostics are a compile-time and tool-time output, not part of a derived component, so tuning
> this choice does not alter any component's bytes.

## The diagnostic record

Each diagnostic is a record with these fields:

| Field | Shape | Meaning |
|---|---|---|
| `code` | string matching `CDZ` followed by four digits (`CDZ0001`) | the stable machine-readable code; stable across unrelated message changes |
| `severity` | one of `error`, `warning`, `note`, `help` | the diagnostic's level |
| `span` | `{ path, start: {line, column, offset}, end: {line, column, offset} }` | the precise source region, by both line/column and byte offset |
| `message` | string | the human-readable summary |
| `rule` | string | the requirement or rule the diagnostic enforces, as a reference of the form `<spec-file>#<section-slug>` |
| `related` | list of `{ span, message }` | secondary spans with explanatory labels |

## Serialization and ordering

- The diagnostic stream is serialized in the canonical value encoding pinned in
  [`hashing-and-encoding`](../hashing-and-encoding/) (deterministic CBOR), with a JSON
  projection for human tools.
- Diagnostics are emitted in a **deterministic order** derived from their spans and codes, so that
  two runs over the same source produce the same diagnostic sequence (diagnostics.md §"Diagnostics
  Are Emitted In A Deterministic Order").

## Why this shape

- A **stable `code`** lets an agent branch on the exact diagnostic without parsing prose, and lets
  the message wording change without breaking automation.
- A **`rule` reference** ties every diagnostic to the requirement it enforces, so an agent can trace
  a rejection to the normative sentence that caused it — the machine-actionable link the
  constitution requires (constitution XI).
- **Byte offsets alongside line/column** let a structural tool locate the span without re-tokenizing.

## The diagnostic-code registry

The stable code is `CDZ` followed by four digits. Because the code is a requirement's machine-readable
identity, the set of codes and each code's meaning is pinned here — the declared-default location —
rather than invented per generation, so that two builds emit the same code for the same rejection and
an executable-semantics `(error CDZ…)` case has a fixed referent. Codes are grouped by the phase and
subject of the rejection; a new rejection is a new code in the appropriate band, never a reuse or a
renumbering of an existing one (a code is stable across message rewordings — diagnostics.md §"Every
Diagnostic Has A Stable Code").

| Code | Meaning (the rejection it names) | Rule it enforces (`<spec-file>#<section-slug>`) |
|---|---|---|
| **`CDZ01xx` — binding, scope, and name resolution** | | |
| `CDZ0101` | reference to a name with no enclosing binding | `spec/capabilities/core-semantics.md#binding-is-lexical` |
| **`CDZ02xx` — types and pattern matching** | | |
| `CDZ0201` | a program that is not well-typed (general type error) | `spec/capabilities/type-system.md#a-well-typed-program-does-not-go-wrong` |
| `CDZ0202` | a value used at a nominal type that is only structurally identical to its declared type | `spec/capabilities/type-system.md#user-types-are-declarable-as-nominal-or-structural` |
| `CDZ0203` | an explicit annotation that conflicts with the type inference determines | `spec/capabilities/type-system.md#annotations-constrain-never-contradict` |
| `CDZ0210` | a match whose patterns do not cover every value of the scrutinee's type | `spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected` |
| **`CDZ03xx` — numeric model** | | |
| `CDZ0301` | an operation on two different numeric types without an explicit conversion | `spec/capabilities/numeric-model.md#numeric-types-do-not-silently-promote` |
| **`CDZ04xx` — capabilities and effects** | | |
| `CDZ0401` | a program that reaches a host operation its manifest does not enumerate | `spec/capabilities/capabilities-and-effects.md#undeclared-capability-is-a-compile-time-error` |

Traps are a distinct, **runtime** category: a trap is not a diagnostic code but a defined-kind halt
carrying a reason string (core-semantics.md §"A Trap Halts Execution At A Defined Point"), witnessed in
the corpus by `(trap "<reason>")`. The reason strings the ignition corpus pins:

| Trap reason | Raised when | Note |
|---|---|---|
| `"integer overflow"` | a checked integer operation overflows its type | numeric-model.md §"Overflow Is Defined" |
| `"list index out of bounds"` | a list is indexed outside its bounds | collections-and-text.md §"List Operations Are Total Or Trap" |
| `"numeric type mismatch"` | the dynamic interpreter evaluates arithmetic on two different numeric types | a typed generation instead rejects at compile time with `CDZ0301` / `CDZ0201` before running |
| `"no matching pattern"` | the dynamic interpreter reaches a `match` whose scrutinee hits no branch | a typed generation instead rejects the non-exhaustive match at compile time with `CDZ0210` before running |
| `"arity mismatch"` | a function is applied to a number of arguments other than the number of parameters it declares | the dynamic interpreter traps at runtime (core-semantics.md §"Applying A Function Binds Its Parameters To Its Arguments"); a typed generation rejects at compile time before running |

The last two are the dynamic seed's runtime halts on programs a typed generation refuses statically;
they are the interpreter primary clause paired with a `(compiler (error …))` annotation in the corpus
(see `spec/learnings/2026-07-02-seed-is-a-dynamic-interpreter.md`,
`spec/semantics/README.md` §"Which cases a generation runs").

The three codes the pre-existing corpus already references — `CDZ0202`, `CDZ0210`, `CDZ0301` — keep
their numbers; the registry adds `CDZ0101`, `CDZ0201`, `CDZ0203`, and `CDZ0401` for the rejections the
ignition witnessing cases exercise.
