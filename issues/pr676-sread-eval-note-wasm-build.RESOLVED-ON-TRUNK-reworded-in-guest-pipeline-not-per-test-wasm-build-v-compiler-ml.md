# pr676 — sread-eval.cdz NOTE says each run-src @test is a "full-pipeline wasm build" (it's the in-guest pipeline)

Mirrored from GitHub PR #676 review comment (Copilot), id 3614143275.
PR: https://github.com/camshaft/cadenza/pull/676 (compiler-ml dup-drop)
Location: `implementation/compiler-ml/src/sread-eval.cdz:262`

## Reviewer comment (verbatim)
> The NOTE says each `run-src` `@test` is a "full-pipeline wasm build", but `cdz test` compiles the test
> file once into a test component; the repeated cost here is the in-guest `run-src` pipeline (read-source →
> resolve → infer → lower → eval). Reword to avoid the inaccurate implication that every `@test` triggers a
> separate wasm build.

## VERIFIED (git show trunk)
sread-eval.cdz:258-262 NOTE (explaining why a byte-identical duplicate `run-src("42")` @test was removed):
"...each run-src @test is a full-pipeline wasm build; the suite time was head-of-line-blocking the fleet
gate." Copilot's correction is accurate to the `cdz test` model: the test FILE compiles ONCE to a test
component; each `run-src` @test then runs the compiler-ml PIPELINE IN-GUEST (read-source→resolve→infer→
lower→eval) at test time — that in-guest pipeline is the repeated cost, not a separate per-@test wasm build.
The dedup rationale (cut repeated run-src cost) is CORRECT; only the "wasm build" phrasing overstates it.
Reword to "each run-src @test re-runs the full in-guest compile pipeline (read→resolve→infer→lower→eval)".
Doc-only, no behavior change.

## Owner
`implementation/compiler-ml/src/sread-eval.cdz` = v-compiler-ml (PORT source; liaison-routing: compiler-ml
source → port owner). Doc/code-shape reword.
