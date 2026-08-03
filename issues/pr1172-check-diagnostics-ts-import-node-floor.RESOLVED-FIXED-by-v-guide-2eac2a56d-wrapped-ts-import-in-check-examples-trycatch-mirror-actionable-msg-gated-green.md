# PR #1172 review comment — guide/scripts/check-diagnostics.mjs (v-guide)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1172
(PR: "cand: v-guide — SizedIntegers + check-diagnostics (touches checks.yml)").

## New gate script fails opaquely on Node 20.19 (advertised floor) for `.ts` import (Copilot, check-diagnostics.mjs:25) — robustness/CI
> This script imports `src/components/wrapModule.ts` directly, which requires Node's TypeScript
> type-stripping. Since `package.json` advertises support for Node >=20.19, running
> `npm run check:diagnostics` on Node 20.19 without `--experimental-strip-types` will fail with the
> opaque "Unknown file extension .ts" loader error. Mirror `check-examples.mjs` by catching this
> import failure and printing an actionable message (Node >=22.6 or the `--experimental-strip-types`
> invocation).

Real portability point on the NEW diagnostic-conformance gate: it imports a `.ts` module directly,
which only works on Node >=22.6 (or with `--experimental-strip-types`), but `package.json` advertises
Node >=20.19 — so a contributor on the floor version hits an opaque "Unknown file extension .ts"
error. `check-examples.mjs` already catches this and prints an actionable message; mirror that here so
the new gate degrades gracefully. (Worth confirming the CI job that runs this gate is on a Node that
type-strips, too.)
