# pr665 — open mlrepro parked in issues/ (should be queue/) + proptest decline msg mislabels heap types as "scalar" (2 Copilot)

Mirrored from GitHub PR #665 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/665 (corpus Map/Set.remove persistence)

## #1 — id 3612308790 (issues/mlrepro-cdz-check-false-cdz0201-...-two-forwarding-hops.cdz) — open repro misfiled [convention]
> This file is an *open* language repro (`mlrepro-*`, describes an active divergence), but repo guidance says
> open repros should live under `.claude/fleet/queue/` and only be archived to `issues/` once resolved.

VERIFIED: the file DOES exist in `issues/` on trunk (unlike the PR#594 mlrepro which had already moved). Per
the compiler-ml README convention (open repros in `.claude/fleet/queue/`, archived to `issues/` when
resolved), this open CDZ0201 const-provability repro is misplaced in the resolved-archive. Fix = relocate to
`.claude/fleet/queue/mlrepro-*.cdz` (or, if actually resolved, mark it .RESOLVED). Process/housekeeping. →
PM to place (the CDZ0201 repro's filer / v-compiler-ml, or just relocate).

## #2 — id 3612308801 (rcdzc/src/proptest_gen.rs:1028) — decline msg calls heap types "scalar" [v-property-testing]
> The decline message describes the unsupported types as a "non-boundary/heap scalar", but the list includes
> both non-boundary scalars (Char) and heap types (String/Symbol) that aren't scalars. Reword to "unsupported
> for property-test generation" and keep the type list.

VERIFIED: proptest_gen.rs:1026 msg = "a parameter's type has no generatable form yet — a non-boundary/heap
scalar (Char/Rational/BigInt/String/Symbol) or a compound with such a leaf...". String/Symbol are HEAP
(not scalars), Char is a non-boundary scalar — "non-boundary/heap scalar" mislabels them. Reword to e.g.
"a type with no property-test-generatable form yet (Char/Rational/BigInt/String/Symbol, or a compound with
such a leaf)". User-facing message clarity. → v-property-testing (owns @property/proptest_gen declines).

## Owner
#1 → PM (repro relocation, fleet convention). #2 → v-property-testing (decline message).

---
## PM disposition (corpus-bugfix, 2026-07-20)
- #1 (CDZ0201 mlrepro misfiled in issues/): v-compiler-ml investigated and RECOMMENDS LEAVING IN PLACE.
  The repro is still OPEN on trunk but is peer-owned (v-core-opt / v-inference, not compiler-ml self-host),
  actively annotated in-place by those verticals (multiple UPDATE blocks reference the current issues/ path).
  A git mv into gitignored queue/ would show as a deletion + break their in-flight references. Relocate only
  once v-core-opt closes the divergence (then it's correctly RESOLVED-in-issues/). Housekeeping nit CLOSED
  as won't-relocate-now-by-consensus. #2 (proptest decline msg) routed to v-property-testing earlier.
