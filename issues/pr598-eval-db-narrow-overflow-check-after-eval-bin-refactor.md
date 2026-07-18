# pr598 — compiler-ml eval-db.cdz: narrow-arith overflow checked AFTER eval-bin (optional refactor)

Mirrored from GitHub PR #598 review comment (Copilot), id 3608982701.
PR: https://github.com/camshaft/cadenza/pull/598 (8-MR publish batch)
Location: `implementation/compiler-ml/src/eval-db.cdz:29` (the `Core.CBin` arm)

## Reviewer comment (verbatim)
> Overflow checking is currently applied *after* `eval-bin(op, a, b)` computes the result. For narrow
> arithmetic this still computes `a + b` / `a - b` / `a * b` in host Int64 semantics before the overflow
> guard runs, and it duplicates the overflow policy in multiple places. Prefer using
> `int-width.checked-*` for narrow +/−/* so overflow maps directly to `Option.None` and the policy stays
> centralized.

## VERIFIED (git show trunk) — NOT a bug, an optional refactor
The `Core.CBin` arm computes `eval-bin(op, a, b)` then, for `width < 64`, guards
`if overflows(v, signed, width) then None else Some(v)`. This is CORRECT — the overflow IS caught and
maps to `Option.None` (declines), matching spec §Overflow / CDZ0304. Copilot's point is a REFACTOR, not
a defect: (a) computing in host Int64 first is fine here because this is the INTERPRETER ORACLE ("reads
the VALUE" per the comment) — the host Int64 range comfortably holds a narrow op's inputs+result, so the
post-hoc `overflows` check is sound; (b) the "policy duplicated in multiple places" / "centralize via
int-width.checked-*" is a reasonable maintainability nit IF such a `checked-*` helper exists and the eval
path should share it. No behavioral change either way.

## Owner + disposition
`compiler-ml/src/eval-db.cdz` = v-inference (owns compiler-ml eval/infer). LOW PRIORITY, optional — the
current code is correct; this is a "would-be-cleaner" centralization suggestion. Owner can dismiss in one
glance if the oracle's compute-then-guard is the intended shape (it reads fine as-is). Filed as a note,
not a bug.
