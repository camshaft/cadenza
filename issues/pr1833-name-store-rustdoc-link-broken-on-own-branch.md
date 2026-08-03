# PR #1833 review comments — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1833 (§4c set/resolve slice 3a — store/* prefix).

## 1. rustdoc intra-doc link to `name_store::NameStore::authority_prefix_of` is broken ON #1833's OWN BRANCH (Copilot, effect.rs:85) — doc/CI [VERIFIED — ordering-dependent]
> This rustdoc intra-doc link points to `crate::name_store::NameStore::authority_prefix_of`, but there is
> no `name_store` module / `NameStore` type in cdz-kernel, so the link will be broken (and can fail builds
> if broken-intra-doc-links is denied).
VERIFIED — an ORDERING dependency: `name_store`/`NameStore::authority_prefix_of` DO exist on current trunk
(added by #1829, lib.rs:50 + name_store.rs:88/134), so the link resolves on post-#1829 trunk. BUT #1833's
OWN cand branch (c99b2c47524a) does NOT have the module (0 matches) — it was cut before #1829 landed. So
on #1833's own tree the link `crate::name_store::NameStore::authority_prefix_of` is genuinely broken →
`rustdoc::broken_intra_doc_links` CI risk if #1833 is evaluated/lands before rebasing onto #1829. Now that
#1829 is merged, ensure #1833 rebases onto current trunk (so the link resolves) before it lands — OR the
build risks a broken-link failure. Recommend v-agent-harness rebase #1833 post-#1829 (a same-vertical
ordering they control). LOW-MED (ordering-dependent CI risk).

## 2. Doc says the predicate is used by "the drive loop's partition test" to route store/* (Copilot, effect.rs:100) — doc/verify
> The doc comment says this predicate is used by "the drive loop's partition test" to route `store/*` to a
> name-store handler.
Verify the store/* routing wiring matches the doc (that the drive loop actually partitions store/* via this
predicate) — if the routing isn't wired yet (slice 3a may be predicate-only), soften to "will be used by".
LOW/doc. Fix-forward.
