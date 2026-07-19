# pr622 — guide runner: `deferred` docstrings say "only <no-generator>" but a 2nd deferred path exists (2 Copilot)

Mirrored from GitHub PR #622 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/622 (4-MR publish batch)
Files: `guide/src/runner/runWorker.ts` + `guide/src/runner/client.ts` — guide browser test-runner infra.

## Comment 1 — id 3610021185 (runWorker.ts:64) — TestResult doc "only"
> The comment says `deferred` is reported *only* when the compiler can't synthesize a generator for the
> parameter shape, but this file also sets `deferred: true` in other cases (e.g. when invoking a test hits
> an unanswered `Test.gen`/`Test.gen-int` host op, or when the compound driver encounters gen-op/name
> drift). Please broaden the doc so it matches actual `deferred` semantics.

## Comment 2 — id 3610021193 (client.ts:198) — runTests doc "only"
> This docstring claims a `deferred` entry appears *only* when a parameterized `@test` has no synthesized
> generator. In practice, the worker may also return `deferred` when a gen host op is unanswered / miswired
> (e.g. name drift or a `-gen` wrapper slipping into the nullary list). Consider loosening "only" to avoid
> baking in an invariant the runtime doesn't strictly enforce.

## VERIFIED (git show trunk)
`runWorker.ts` has TWO `deferred: true` sites: line 225 (`"property test — deferred (needs generated
inputs)"` — the no-synthesized-generator case the doc describes) AND line 510 (`deferred: true, error:
"property test — deferred (${message...})"` — a broader error path, the unanswered/miswired gen-op case
Copilot names). So the docstrings at runWorker.ts:62 and client.ts:196 that say `deferred` is reported
"only" for the no-generator case are inaccurate — there's a second path. Fix = loosen "only" / add the
second cause. Doc-comment accuracy only, no behavior change.

## Owner
`guide/src/runner/*` = guide browser test-runner infra → v-guide-editor (area=guide). Both fold into one
doc edit.
