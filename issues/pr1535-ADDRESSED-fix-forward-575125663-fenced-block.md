# PR #1535 review comment — fleet/AGENTS-fleet.md (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1535 (PR: "[v-fleet-tooling] 08174047d").

## Nested backticks break the example code span (Copilot, AGENTS-fleet.md:255) — doc/rendering
> The example root-line format uses an inline code span that contains nested backticks around `v-x`,
> which will break Markdown rendering (the inner backticks terminate the outer code span). Wrap the
> whole example in a double-backtick code span (or use a fenced code block) so the inner `v-x` stays
> intact.

Use a double-backtick span (`` `` ``) or a fenced block for the example root-line so the inner
`v-x` backticks don't terminate the outer span and mangle the render.
