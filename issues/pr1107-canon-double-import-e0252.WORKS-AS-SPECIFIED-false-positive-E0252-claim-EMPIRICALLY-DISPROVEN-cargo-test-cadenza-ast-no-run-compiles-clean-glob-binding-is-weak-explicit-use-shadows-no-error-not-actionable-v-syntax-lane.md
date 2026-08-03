# PR #1107 review comment — cadenza-ast/src/canon.rs (v-syntax)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1107
(PR: "cand: v-syntax — canon.rs (oldest-first)").

## Claimed double-import E0252 (Copilot, canon.rs:233) — LOW-CONFIDENCE, likely false-positive, verify
> The test module imports `Arenas` and `StructId` twice: they are already brought into scope via
> `use super::*;` (because `canon.rs` imports them at module scope), so
> `use crate::ast::{Arenas, Builder, StructId};` will trigger E0252 ("name is defined multiple
> times") and fail to compile. Import only what isn't already in `super::*` (e.g. just `Builder`), or
> make the `super` import explicit instead of globbing.

⚠ Likely a FALSE POSITIVE on the "fail to compile" claim: the PR gated GREEN, so there is no live
E0252 — a `use super::*;` glob does NOT conflict with an explicit `use` of the same item unless the
glob actually re-exports it AND Rust considers them distinct-origin (a glob-imported name is a weak
binding that an explicit `use` shadows without error). If `canon.rs` module scope does NOT
`pub use`/`use` those names such that the glob re-imports them, there's no conflict. NOT actionable
as a compile fix. Only worth a look if v-syntax wants to tidy a genuinely-redundant import — but do
NOT change it on the E0252 claim alone (it compiles).
