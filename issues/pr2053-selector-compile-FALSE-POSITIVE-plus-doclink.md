# PR #2053 review — cdz-kernel/src/selector.rs (v-agent-harness) — MERGED — 1 FALSE-POSITIVE (dismiss) + 1 LOW doc-link

https://github.com/camshaft/cadenza/pull/2053 (selector — artifact output-routing, invoke slice 2).
Copilot 2 inline: a "won't compile" claim that's a FALSE POSITIVE, and a real broken doc-link.

## `SelectorRule::matches` "`*k` moves a `String` out of a shared ref, won't compile" (Copilot id 3713776806) — DISMISS, FALSE POSITIVE [VERIFIED]
> `SelectorRule::matches` dereferences `&String` as `*k`, which attempts to move a `String` out of a
> shared reference and will not compile. Compare by reference instead.

FALSE POSITIVE. The line (selector.rs:57) is `self.kind.as_ref().is_none_or(|k| *k == artifact.kind)`.
`self.kind: Option<String>` → `.as_ref()` → `Option<&String>` → `k: &String`. `*k == artifact.kind` does
NOT move: `==` is `PartialEq::eq(&self, &other)`, which takes both operands by REFERENCE — the `*k` is
auto-re-borrowed for the comparison (standard `String == String` via the `impl`). No move out of the shared
ref occurs. PROOF: #2053 MERGED GREEN — CI compiled it. Copilot mistook a deref-in-comparison for a move.
DISMISS. (Verify-before-relay: a "won't compile" claim on already-MERGED code is refuted by the merge
itself; confirmed the deref semantics rather than relaying.)

## intra-doc link `crate::authz::ResourcePredicate::Prefix` is wrong — `ResourcePredicate` lives in `crate::effect` (Copilot id 3713776860) — doc-link [VERIFIED, LOW]
> This intra-doc link points at `crate::authz::ResourcePredicate::Prefix`, but `ResourcePredicate` is
> defined under `crate::effect` (and `authz` reuses it). As written, the link will be broken in rustdoc.

VERIFIED: selector.rs:40 doc has `[`crate::authz::ResourcePredicate::Prefix`]`, but `ResourcePredicate` is
`pub enum` in `effect.rs:361` (authz merely re-uses it). So the rustdoc intra-doc link resolves to a
nonexistent `authz::ResourcePredicate` → broken link (and `cargo doc` may warn/error under
`-D rustdoc::broken_intra_doc_links` if enabled). LOW/doc-link. Fix: `[`crate::effect::ResourcePredicate::
Prefix`]`. v-agent-harness owns cdz-kernel/src.
