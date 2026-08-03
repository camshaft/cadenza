# PR #1058 review comment — rcdzc/src/infer.rs (v-diagnostics)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1058
(PR: "cand: v-diagnostics — bare width-ctor default type (infer.rs + tests.rs)").

## `bare_width_ctor_default_type` computed twice (Copilot, infer.rs:1681) — simplification
> `bare_width_ctor_default_type(db, ty_expr)` is computed twice (in the match guard and again for
> `unwrap()`), which repeats metadata projection work and makes the control flow harder to read.
> Consider collapsing the width-ctor and fallback cases into a single `Some(name)` arm that calls
> `bare_width_ctor_default_type` once and branches on the result.

Non-blocking quality point on the new CDZ0203 messaging path.
