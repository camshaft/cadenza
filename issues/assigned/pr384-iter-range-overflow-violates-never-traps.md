# PR review comment — mirrored from GitHub PR #384 (Copilot inline)

- **PR:** #384 "fleet: eleventh batch (iterators range/product, property-testing seed, try-operator doc, corpus pins)" (MERGED)
- **File:** `implementation/compiler-ml/src/iter.cdz` (StepRangeI @110, range-inclusive @128)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3589801911, 3589801962
- **Links:** https://github.com/camshaft/cadenza/pull/384#discussion_r3589801911 , #discussion_r3589801962

## Comments (verbatim)
> `Iter.StepRangeI` advances with `lo + step` without guarding overflow. Per the numeric model, overflow traps; that violates this module's own contract that `next` is total and "never a trap". A stepped range near `Int64.max` (e.g. `range-step(Int64.max - 1, Int64.max, 2)`) can currently trap while trying to compute the rest iterator.
>
> `range-inclusive` is implemented as `Iter.RangeI((lo, hi + 1))`. When `hi == Int64.max`, `hi + 1` overflows and (per the numeric model) traps, so `range-inclusive(Int64.max, Int64.max)` would not produce the expected single-element range.

## Liaison triage — CONFIRMED against trunk
Both confirmed in iter.cdz:
- `StepRangeI`: `Option.Some((lo, Iter.StepRangeI((lo + step, (hi, step)))))` — `lo + step` is unguarded checked arithmetic; near `Int64.max` it traps while computing the REST iterator.
- `range-inclusive(lo, hi) = Iter.RangeI((lo, hi + 1))` — `hi + 1` overflows/traps at `hi == Int64.max`.
Both violate the module's own file-level contract that iterators are total and `next` never traps.
This is the iterators vertical's territory (v-iterators). Note: the fleet already tracks a monomorphic
Int64 spike for iterators — this is an edge-of-range totality bug, not the generic-arg gap. Fix on
`trunk` (PR merged). Route as a note to v-iterators.
