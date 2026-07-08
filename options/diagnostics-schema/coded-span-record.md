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
| `fix` | a structural edit of the program's binary AST (the ast-encoding edit shape), or absent | the proposed route to a compliant program — a structural AST edit, not a textual patch (diagnostics.md §"A Rejection Carries A Structural Fix") |
| `fix_status` | `verified`, or `heuristic-<applicability>` where `<applicability>` is one of `maybe-incorrect`, `has-placeholders` | whether the compiler confirmed the fix recompiles clean and clears the diagnostic (`verified`) or could not, in which case it declares the fix a heuristic and carries the applicability marker an agent branches on (diagnostics.md §"A Confirmed Fix Is Marked Verified", §"An Unconfirmed Fix Carries An Applicability Marker") |

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
| **`CDZ00xx` — reader (lexical / literal syntax)** | | |
| `CDZ0001` | a string literal containing an unrecognized escape sequence — a backslash before a character that begins none of the recognized escapes (`\n`, `\t`, `\r`, `\\`, `\"`) | `spec/capabilities/collections-and-text.md#a-string-literals-escapes-are-a-closed-set` |
| `CDZ0002` | a char literal that names a value which is not a Unicode scalar — a code point in the surrogate range `U+D800..=U+DFFF` or above `U+10FFFF` — so that a char literal can never denote a value that is not a valid scalar | `spec/capabilities/collections-and-text.md#a-char-is-a-single-unicode-scalar-value` |
| **`CDZ01xx` — binding, scope, and name resolution** | | |
| `CDZ0101` | reference to a name with no enclosing binding | `spec/capabilities/core-semantics.md#binding-is-lexical` |
| `CDZ0102` | a pattern that binds the same name more than once | `spec/capabilities/core-semantics.md#bindings-introduced-by-a-pattern-are-scoped-to-its-branch` |
| **`CDZ02xx` — types and pattern matching** | | |
| `CDZ0201` | a program that is not well-typed (general type error) | `spec/capabilities/type-system.md#a-well-typed-program-does-not-go-wrong` |
| `CDZ0202` | a value used at a nominal type that is only structurally identical to its declared type | `spec/capabilities/type-system.md#user-types-are-declarable-as-nominal-or-structural` |
| `CDZ0203` | an explicit annotation that conflicts with the type inference determines | `spec/capabilities/type-system.md#annotations-constrain-never-contradict` |
| `CDZ0210` | a match whose patterns do not cover every value of the scrutinee's type | `spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected` |
| `CDZ0211` | a record row operation that would place two values under one field name — combining two records whose field sets share a name, or adding a field the operand record already contains — rejected so the combined record need not choose which value a shared field takes (the row-operation companion of the duplicate-field-literal case of `CDZ0201`) | `spec/capabilities/type-system.md#two-records-are-combined-only-when-their-field-sets-are-disjoint` |
| `CDZ0212` | a record row operation that names a field the operand record does not contain — projecting onto, dropping, or updating an absent field — rejected so the operation cannot silently produce, no-op, or add a field the operand never held | `spec/capabilities/type-system.md#a-record-is-restricted-to-a-named-set-of-its-fields` |
| `CDZ0220` | a binary form that is not byte-aligned or is otherwise ill-formed — bit-field widths that do not close a byte, a non-final unsized bytes segment, or a bit-field width that is not a compile-time constant | `options/binary-syntax/README.md` |
| `CDZ0221` | a quote pattern that is ill-formed — an unquote-splicing `,@` subterm that is not the final element of its template, so it would match a variable-length gap flanked by a fixed tail | `options/quote-patterns/README.md` |
| **`CDZ03xx` — numeric model** | | |
| `CDZ0301` | an operation on two different numeric types without an explicit conversion | `spec/capabilities/numeric-model.md#numeric-types-do-not-silently-promote` |
| `CDZ0302` | an integer type indexed by a width outside the admitted range — e.g. `(UInt 0)`, `(UInt 65)`, a negative width, or a non-natural width — a specialization of the compile-time-constraint rejection (type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values") for the integer width constructor | `spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width` |
| `CDZ0303` | a `(pragma default-integer <T>)` module directive whose `<T>` is not an integer type the numeric model admits — e.g. a float, a rational, or a non-numeric type (the numeric-domain check on a well-formed directive, distinct from the structural `CDZ0602`) | `spec/capabilities/numeric-model.md#a-module-may-declare-its-default-integer-literal-type` |
| **`CDZ04xx` — capabilities and effects** | | |
| `CDZ0401` | an effect operation reached at a point with neither an enclosing handler for its effect nor an enclosing entrypoint delegation of it — so the effect would escape ungranted. This is the single "no home for a reached effect" rejection: it subsumes both the former reached-but-undelegated host operation and the former undischarged intra-program effect, which are one condition now that host-binding is an entrypoint routing decision rather than a declaration-time property | `spec/capabilities/capabilities-and-effects.md#an-ungranted-effect-is-a-compile-time-error` |
| `CDZ0402` | *(merged into `CDZ0401`; retained as a reserved number.)* Formerly the undischarged intra-program effect, distinct from the reached-but-undeclared host operation. With host-binding relocated from the declaration to the entrypoint, the two collapsed into `CDZ0401` — an effect reached an entrypoint's top with no handler and no delegation. Not emitted | `spec/capabilities/capabilities-and-effects.md#an-ungranted-effect-is-a-compile-time-error` |
| `CDZ0403` | a handler arm that names an operation the arm's effect does not declare, since an effect declaration is the closed set of its operations | `spec/capabilities/capabilities-and-effects.md#a-handler-arm-names-an-operation-its-effect-declares` |
| `CDZ0404` | an entrypoint host delegation that names an effect the delegated computation never reaches, so the manifest would carry latent authority — a granted capability that is never exercised — rejected so the manifest is exactly the effects that escape, no more and no fewer | `spec/capabilities/capabilities-and-effects.md#host-delegation-is-an-entrypoints-prerogative` |
| **`CDZ05xx` — verification layers (dimensional analysis, refinements, contracts, proofs)** | | |
| `CDZ0501` | a combination of quantities whose dimensions are incompatible — adding, subtracting, or comparing quantities of unlike dimension, or annotating a quantity at a dimension the operation does not derive | `spec/capabilities/units-of-measure.md#dimensional-mismatch-is-an-error` |
| **`CDZ06xx` — module directives (pragmas)** | | |
| `CDZ0601` | a module directive whose pragma key is not one the pinned registry defines — an unrecognized directive, which is rejected rather than ignored so it can neither silently change nor silently fail to change a program's meaning | `spec/capabilities/modules-and-namespaces.md#an-unrecognized-module-directive-is-rejected` |
| `CDZ0602` | a module directive whose pragma key is recognized but whose arguments do not match the shape that key defines — e.g. a wrong number of arguments or an argument of the wrong kind (distinct from a well-formed directive whose argument violates a domain rule, which carries that domain's code) | `spec/capabilities/modules-and-namespaces.md#a-module-directive-is-drawn-from-a-fixed-set` |

Traps are a distinct, **runtime** category: a trap is not a diagnostic code but a defined-kind halt
carrying a reason string (core-semantics.md §"A Trap Halts Execution At A Defined Point"), witnessed in
the corpus by `(trap "<reason>")`. The reason strings the ignition corpus pins:

| Trap reason | Raised when | Note |
|---|---|---|
| `"integer overflow"` | a checked integer operation overflows its type | numeric-model.md §"Overflow Is Defined" |
| `"division by zero"` | an integer division or modulo has a zero divisor, which has no quotient or remainder | core-semantics.md §"Partial Operations Have A Defined Outcome" |
| `"rational with zero denominator"` | an exact rational is constructed with a zero denominator, which denotes no number | numeric-model.md §"A Rational With A Zero Denominator Is Not A Value" |
| *the message the `expect` supplied* | `expect` is applied to an absent optional; the trap reason is verbatim the mandatory message the `expect` call carries, so the halt names the requirement the program stated rather than the upstream access | core-semantics.md §"Requiring The Value Of An Optional Traps On Absence" |
| `"byte value out of range"` | a `Bytes` value is constructed from an integer outside `0..=255` | self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous" (the seed-realized `Bytes` form; `options/realized-capability-set/`) |
| `"binary value does not fit segment"` | a `(bin …)` construction is given a value with no encoding in its segment — a value above an unsigned segment's range, a negative value in an unsigned segment, or a value wider than a bit-field's width | `options/binary-syntax/README.md` (total-or-trap `bin` construction) |
| `"member access on a non-record"` | member access `(. v k)` is applied to a value `v` that is not a record | core-semantics.md §"Member Access Projects A Record Field" |
| `"no such field"` | member access `(. r k)` names a field `k` the record `r` does not contain | core-semantics.md §"Member Access Projects A Record Field" |

Several conditions that a dynamic evaluator would have trapped on at runtime — arithmetic on two
different numeric types, a non-exhaustive `match`, and applying a function to the wrong number of
arguments — are **not** runtime traps under the seed: the seed is a compiler, so it rejects these at
compile time with a diagnostic code (`CDZ0301`/`CDZ0201`, `CDZ0210`, and the arity/ill-typedness codes
respectively) before the program runs (constitution §VII; Amendment 0.4.0). In the corpus these are
the `(compiler (error …))` clauses the seed produces
(see `spec/learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md`,
`spec/semantics/README.md` §"Which cases a generation runs"), not `(trap …)` reasons.

The three codes the pre-existing corpus already references — `CDZ0202`, `CDZ0210`, `CDZ0301` — keep
their numbers; the registry adds `CDZ0101`, `CDZ0201`, `CDZ0203`, and `CDZ0401` for the rejections the
ignition witnessing cases exercise. `CDZ0402` is a **reserved, no-longer-emitted** number: it was the
undischarged-intra-program-effect rejection until host-binding moved from an effect's declaration to an
entrypoint's delegation, at which point it merged into `CDZ0401` (an effect reached an entrypoint's top
with neither a handler nor a delegation). `CDZ0404` is added for latent authority — a delegation naming
an effect that is never reached.
