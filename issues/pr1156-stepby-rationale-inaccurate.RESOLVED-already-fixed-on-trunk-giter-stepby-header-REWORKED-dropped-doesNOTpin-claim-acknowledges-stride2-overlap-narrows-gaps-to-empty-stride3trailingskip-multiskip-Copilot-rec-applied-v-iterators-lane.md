# PR #1156 review comment — implementation/iterators/src/giter-stepby.cdz (v-iterators)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1156
(PR: "cand: v-iterators — giter-stepby").

## Module header rationale contradicts existing giter.cdz coverage (Copilot, giter-stepby.cdz:14) — doc
> The header comment claims `giter.cdz` only tests step-by on lengths that end on a kept element and
> "does NOT pin" mid-stride exhaustion, but `giter.cdz` already covers trailing-skip exhaustion for
> stride 2 (see `step-by-keeps-every-second-int` over 6 elements at
> implementation/iterators/src/giter.cdz:865-869). This makes the rationale for the extra mid-stride
> bullet inaccurate/misleading.

The new module's stated rationale ("giter.cdz doesn't pin mid-stride exhaustion") is factually wrong
— `step-by-keeps-every-second-int` (giter.cdz:865-869) already exercises trailing-skip for stride 2.
Either narrow the rationale (e.g. "extends to stride>2 / larger-than-input" if that's the genuine
gap) or drop the inaccurate claim. The added tests are still fine; it's the justifying comment that
overstates the gap.
