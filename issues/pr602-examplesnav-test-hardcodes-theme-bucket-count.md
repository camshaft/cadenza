# pr602 — ExamplesNav.test.ts hardcodes `renderedThemes().size === 4` (contradicts its anti-vacuous intent)

Mirrored from GitHub PR #602 review comment (Copilot), id 3609102465.
PR: https://github.com/camshaft/cadenza/pull/602 (8-MR publish batch)
Location: `guide/src/components/ExamplesNav.test.ts:74`

## Reviewer comment (verbatim)
> This test says it derives the rendered theme buckets from ExamplesNav.tsx "rather than hard-coded",
> but then hard-codes `renderedThemes().size === 4`. That will fail on any intentional theme-bucket
> addition/removal, undermining the stated goal. Use a non-vacuous lower bound (or otherwise avoid a
> fixed count) to keep the test aligned with the comment.

## VERIFIED (git show trunk)
The test `the nav data scan found examples + buckets (guards against a vacuous pass)` uses LOWER BOUNDS
for the arrays (`PLAYGROUND_EXAMPLES.length >= 30`, `CAD_EXAMPLES.length >= 5`,
`NOTEBOOK_EXAMPLES.length >= 5`) — consistent with its stated "assert healthy counts so a refactor that
breaks extraction trips here" intent. But the last line is EXACT: `assert.equal(renderedThemes().size, 4,
...)`. So an intentional theme-bucket add/remove fails this test even though nothing broke — contradicting
the "non-vacuous, refactor-resilient" design of the rest of the test. Fix (per Copilot): make it a lower
bound (`renderedThemes().size >= 1` or `>= 2`) like the sibling asserts. Minor test-robustness nit.

## Owner
`guide/src/components/ExamplesNav.test.ts` = v-guide-editor (added this nav-invariant gate test — it's
the "ExamplesNav.test.ts" flagged as active in the guide vertical). area=guide.
