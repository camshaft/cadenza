# PR review comment — mirrored from GitHub PR #391 (Copilot inline)

- **PR:** #391 (OPEN at triage; file on trunk)
- **File:** `guide/scripts/check-examples.mjs:64`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590196782
- **Link:** https://github.com/camshaft/cadenza/pull/391#discussion_r3590196782

## Comment (verbatim)
> This imports a `.ts` module (`src/components/wrapModule.ts`). Unless the script is always run with Node's TypeScript stripping enabled, this will fail with an unhelpful loader error. Consider wrapping the import to throw a clear, actionable message (or switch to importing a `.js` entrypoint).

## Liaison triage
Guide-tooling robustness: `check-examples.mjs` imports a `.ts` module, which fails with a cryptic Node
loader error unless TS stripping is enabled. Since this script is the new pre-merge guide-examples gate
(PR #390 added the CI job), a cryptic failure here would be confusing. Wrap the import to throw an
actionable message, or import a `.js` entrypoint. Guide territory (v-guide). Route as a note. Fix on
`trunk`. Quote + link in queue file.
