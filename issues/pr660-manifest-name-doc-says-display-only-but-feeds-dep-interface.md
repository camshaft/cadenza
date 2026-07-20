# pr660 — manifest `def name` documented "display only" but it feeds the `cadenza:<name>/api` dep interface

Mirrored from GitHub PR #660 review comment (Copilot), id 3612220221.
PR: https://github.com/camshaft/cadenza/pull/660 (cdz CLI stack)
Location: `implementation/seed/crates/cdz/src/main.rs:7756` (doc bullet at :7741)

## Reviewer comment (verbatim)
> The manifest docs list `def name = "…"` as "display only", but the CLI logic uses manifest `name` as the
> dependency interface segment (`cadenza:<name>/api`) with a directory-name fallback (see dep interface
> construction/validation earlier in this file). This mismatch can mislead manifest authors; please update
> the bullet so it reflects the real behavior.

## VERIFIED (git show trunk)
main.rs:7741: `- `def name = "…"`  — the project name (display only).` But the `name` field IS the dep
interface segment: main.rs:2087-2116 builds `cadenza:<dep-name>/api` from the dep's manifest `name` (with a
directory-name fallback, and a lowercase-validation because "a dependency's `def name` becomes
`cadenza:<name>/api`"). The struct doc at 5575-ish even says name is "used for the published
`cadenza:<name>/api` interface segment." So "display only" is inaccurate — `name` is load-bearing for a
dependency's published interface. Fix = reword the bullet to note `name` becomes the `cadenza:<name>/api`
interface segment (must be lowercase), with the directory-name fallback. Doc-only, no behavior change.

## Owner
`implementation/seed/crates/cdz/src/main.rs` manifest = v-cdz-tooling (owns the `cdz` CLI + manifest).
