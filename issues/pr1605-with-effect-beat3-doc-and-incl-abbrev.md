# PR #1605 review comments — implementation/seed/crates/cdz-kernel/src/executor.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1605 (PR: "feat(cdz-kernel):
CompositeExecutor::with_effect(family, exec) — the register-by-string registration API (beat 1b)").
Two LOW nits.

## 1. `with` doc carries a time-bound "beat 3" migration note (Copilot, executor.rs:67) — doc/durability
> The doc comment for `with` includes time-bound migration notes ("beat 3") that are likely to become
> stale and conflict with the repo guideline to keep comments describing current behavior rather than
> the construction/migration sequence.

VERIFIED against the diff: the `with` doc ends "(… register-by-string bridge, beat 3.)". Same
durability pattern this liaison has flagged on #1554/#1573/#1575 — a migration-beat reference in a
durable code comment. Describe current behavior (this is the legacy `EffectKind` registration path
being superseded by `with_effect`) without the "beat 3" sequencing tag. LOW.

## 2. Test name uses `incl` abbreviation vs spelled-out neighbors (Copilot, executor.rs:283) — style
> Test name uses the abbreviation `incl`, while nearby tests spell words out fully; expanding it
> improves consistency and readability in test output.

VERIFIED: `with_effect_registers_by_family_string_incl_an_extension_family` — expand `incl` →
`including` to match neighboring test naming. LOWEST/style.
