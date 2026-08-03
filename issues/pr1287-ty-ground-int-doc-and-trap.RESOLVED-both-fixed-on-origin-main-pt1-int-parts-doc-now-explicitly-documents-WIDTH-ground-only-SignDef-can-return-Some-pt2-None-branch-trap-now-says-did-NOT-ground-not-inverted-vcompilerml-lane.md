# PR #1287 review comments — implementation/compiler-ml/src/ty.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1287 (PR: "cand: v-compiler-ml — cf12f5af5").

## 1. Docstring says "ground" int but helper returns `Some` for `SignDef` (Copilot, ty.cdz:203) — doc/correctness
> The docstring says this helper extracts parts of a *ground* int type, but the implementation
> returns `Some((s, n))` for `WFixed(n)` even when `s` is `SignDef` (which is not ground per
> `is-ground-int`). Either document that `SignDef` can be returned, or treat deferred sign as `None`
> to match the "ground" wording.

Doc-vs-behavior: pick one — either treat a deferred `SignDef` as `None` (so it truly only returns
ground parts, matching the wording), or update the doc to state `SignDef` can come back.

## 2. Inverted trap/failure message on the None branch (Copilot, ty.cdz:216) — diagnostics
> This failure message is inverted: the `Option.None` branch indicates the deferred int did *not*
> ground to a fixed-width int (or even an int), so the trap text should reflect that to make test
> failures diagnosable.

The `None` branch means "did NOT ground to a fixed-width int", but the message says the opposite —
correct it so a failing case is diagnosable.
