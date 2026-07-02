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
  [`hashing-and-encoding.md`](./hashing-and-encoding.md) (deterministic CBOR), with a JSON
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
