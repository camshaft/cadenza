# PR #1166 review comment — guide/src/content/arc.test.ts (v-guide-editor)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1166
(PR: "cand: v-guide-editor — arc.test.ts (oldest disjoint)").

## Regex catastrophic-backtracking risk on malformed registry (amazon-q, arc.test.ts:153) — robustness
> The pattern `/slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g` with `[\s\S]*?` can
> cause exponential time complexity if the registry file contains malformed entries without matching
> imports. If a slug exists but its import is malformed or missing, the regex engine will backtrack
> through the entire remaining file content for each exec() call.
>
> Replace with a more constrained pattern that limits the search space between slug and import, or
> use a two-pass approach to avoid backtracking across the entire file.
> suggested: `/slug:\s*"([^"]+)"[\s\S]{0,500}?import\("\.\/chapters\/([^"]+)"\)/g`

Severity is modest (this is a test/build-time scan over a repo-controlled registry, not
attacker-controlled input), but the fix is cheap: bound the gap between `slug:` and `import(` (e.g.
`{0,500}?`) or split into a two-pass parse so a malformed/missing import can't make the engine
backtrack across the whole file.
