# PR #1577 review comment — implementation/seed/crates/cdz-kernel/src/effect.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1577 (PR: "[v-agent-harness] d7241e4c3").
This changes `EffectRequest.target` from `String` to `Arc<str>` (operator Bytes/cheap-clone directive
— O(1) refcount clone as the effect threads dispatch→authz→executor).

## Test comment still says `impl Into<String>` after switch to `impl Into<Arc<str>>` (Copilot, effect.rs:401) — doc/accuracy
> The test comment still refers to `impl Into<String>` for `EffectRequest::new`'s `target` parameter,
> but this PR changes it to `impl Into<std::sync::Arc<str>>`. Updating the comment avoids misleading
> future readers about the API contract.

VERIFIED against the diff: the signature changed `target: impl Into<String>` → `target: impl
Into<std::sync::Arc<str>>` (and the field `pub target: String` → `Arc<str>`), but a test comment
around effect.rs:401 still references the old `impl Into<String>` contract. Update it to `Into<Arc<
str>>`. Doc-only, LOW. (The prod-doc on the field itself was updated correctly — "`EffectRequest::new`
takes `impl Into<Arc<str>>` so `&str`/`String` call sites are unaffected" — so it's just the test
comment that lagged.)
