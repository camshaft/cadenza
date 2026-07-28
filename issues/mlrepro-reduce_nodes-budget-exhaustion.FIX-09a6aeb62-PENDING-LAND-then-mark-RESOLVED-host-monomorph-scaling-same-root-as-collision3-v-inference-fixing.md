# mlrepro: host emits spurious unlocated CDZ0201 when many whole-pipeline `run-src` test-components share a heavier `lower-db`

**Reporter:** v-compiler-ml · **Date:** 2026-07-22 · **Severity:** medium (blocks a self-host slice; not a wrong-value miscompile)
**Component:** `rcdzc` host compiler — `cdz test` wasm-component build / monomorphization stage (NOT the type checker).

## Symptom

Building `implementation/compiler-ml/src/sread-eval-fns.cdz` (36 `@test`s, each calls `run-src` which
compiles the WHOLE compiler-ml pipeline into its own wasm test-component) with a slightly heavier `lower-db`
module in the import closure produces **103 diagnostics** of the form, with **NO source location**:

```
cdz: error [CDZ0201]: member access requires a record, found Type
cdz: error [CDZ0201]: a Option value has no field `Some` — a sum's payload is reached by matching its variants...
cdz: error [CDZ0201]: a Option value has no field `None` — ...
cdz: error [CDZ0203]: the type position of an annotation requires a type, but found a non-type
```

The same source **passes `cdz check` cleanly** (no type error) and the heavier `lower-db` **passes its own
`cdz test` at 0/0**. The errors appear ONLY at the `cdz test` (wasm-component build) stage, and ONLY when
MANY distinct whole-pipeline components are built in one file.

## Why this looks like a host bug, not user-code

Bisected by holding the source fixed and varying only the number/diversity of run-src components:

| Configuration | Result |
|---|---|
| `cdz check sread-eval-fns.cdz` (+ heavier lower-db) | CLEAN (0 errors) |
| `cdz test lower-db.cdz` (the heavier module, standalone) | 0/0 PASS |
| 1 distinct run-src test + heavier lower-db | PASS |
| 8 IDENTICAL run-src tests + heavier lower-db | 8/0 PASS |
| 6 DISTINCT run-src programs + heavier lower-db | 6/0 PASS |
| 36 distinct run-src tests + heavier lower-db (real file) | **103 spurious CDZ0201, unlocated** |
| 36 distinct run-src tests, baseline lower-db (trunk) | 36/0 PASS |

So: the code is well-typed (`cdz check` clean), the heavier module compiles alone, and few components build
fine — the errors emerge only from the COMBINATION of (heavier per-component monomorphization) × (many
distinct whole-pipeline components in one file). A source-error would have a location and would fail `cdz
check`; this fails only the later build stage with no location. That is a host monomorphization/emit
scaling limit, surfacing as misattributed CDZ0201/0203 (record/sum field access on a `Type`).

## The "heavier lower-db" (the trigger weight)

The Slice-B1b `lower-def-env` chain added to `implementation/compiler-ml/src/lower-db.cdz`: helpers
`def-param-ids` / `def-param-scope` / `seed-param-types` / `lower-one-def` / `build-def-env` /
`lower-def-env`, which newly import `resolve-node`+`param-scope` (resolve-db) and `infer-node` (infer-db)
into lower-db to lower one def body standalone, and use the type `Map(Int64, Tuple(List(Int64), Core))`.
NOTE: this chain is DEAD CODE in the run-src closure at the tested revision (run-of-db does not call it yet)
— its mere presence in the module (hence in each test-component's monomorphized closure) is enough to tip
the 36-component build over. Swapping the named `Tuple(...)` for the anonymous `(...)` tuple type did NOT
change the outcome (both forms trip it); removing `Map.to-list` did NOT change it. So it is aggregate module
weight, not one construct.

## Reproduction recipe (needs the v-compiler-ml Slice-B1b branch)

1. Apply the B1b stack onto trunk (commits `df57ef2ef` resolve-db export, `9868dc358` infer-db if-type join,
   `e4ca71983`+`e5d22b86c` lower-db `lower-def-env`). Or just add the `lower-def-env` chain to `lower-db.cdz`.
2. `target/release/cdz check implementation/compiler-ml/src/sread-eval-fns.cdz` → CLEAN.
3. `target/release/cdz test implementation/compiler-ml/src/lower-db.cdz` → 0/0 PASS.
4. `target/release/cdz test implementation/compiler-ml/src/sread-eval-fns.cdz` → ~103 unlocated CDZ0201.
5. Delete tests down to ~6 distinct run-src programs → PASS. So it is a COUNT/weight threshold, ~between 6 and 36.

## Ask

- Confirm this is a host monomorphization/component-build scaling limit (not a real type error) and, if so,
  give the diagnostics a **source location** (or a distinct diagnostic) so it is not misreported as CDZ0201
  "member access requires a record, found Type". Silent, locationless errors at the build stage read as a
  self-host bug in the .cdz code when they are a host limit.
- Ideal fix: raise/remove the per-file cumulative component-build limit, or make monomorphization of a shared
  heavy module across many components not multiply cost the way it appears to here.

## v-compiler-ml workaround path (does NOT need this fixed to proceed)

The existing codebase already SPLITS run-src test files to stay under the per-file build budget (sread-eval-fns
was itself split out of sread-eval for the 360s timeout). When the B1b p3 wiring lands, I can split
sread-eval-fns into two smaller files so each stays under the component-count threshold — the same idiomatic
pattern already in use. Filing this anyway per the operator "surface friction" directive, because the
LOCATIONLESS-CDZ0201 misreport is a real host diagnostic-quality bug independent of my slice.
