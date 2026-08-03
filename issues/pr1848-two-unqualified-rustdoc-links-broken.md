# PR #1848 review comments — cdz-kernel/src/{authz,effect}.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1848 (§4c — Capability::for_family + Authorizer family grants).
Two rustdoc broken-intra-doc-link risks (recurring #1692/#1764/#1815 pattern).

## 1. `[`EffectKind`]` unqualified at authz.rs:45 — not in module scope (Copilot, authz.rs:45) — doc/rustdoc [VERIFIED]
> The `family_grants` field doc references `[`EffectKind`]` but EffectKind isn't imported into this module
> — broken intra-doc link. Fully-qualify (or import).
VERIFIED: the link `[`EffectKind`]` is at authz.rs:41, but `use crate::effect::{EffectKind, ...}` only
appears at :99 inside `#[cfg(test)] mod tests` — so at module-doc level EffectKind isn't in scope → broken
link. Notably :59 in the SAME file already uses the correct fully-qualified `[`crate::effect::
EffectKind`]` — so just make :41 match :59's qualified form. LOW/doc.

## 2. `[`Authorizer::with_family_grants`]` unqualified at effect.rs:413 — Authorizer not in scope (Copilot, effect.rs:413) — doc/rustdoc [VERIFIED]
> This rustdoc link uses `Authorizer::with_family_grants` without a module path; Authorizer isn't in scope
> in effect.rs, so it'll likely be a broken intra-doc link. Use a fully-qualified path.
VERIFIED: `[`Authorizer::with_family_grants`]` at effect.rs:413, but `use crate::authz::Authorizer` only
appears inside test fns (:972+) — not at module scope. Qualify to `[`crate::authz::Authorizer::
with_family_grants`]`. LOW/doc. Both fix-forward (or before-land — a denied broken_intra_doc_links lint
would red the doc job).
