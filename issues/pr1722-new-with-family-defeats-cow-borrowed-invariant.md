# PR #1722 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1722 (add EffectRequest::new_with_family).

## `new_with_family` allocates even for well-known families — defeats the Cow::Borrowed zero-alloc invariant (Copilot, effect.rs:258) — perf/consistency
> `new_with_family` forces an allocation even for well-known families: it takes `Into<Arc<str>>`,
> allocates an `Arc`, then converts to `String` to store `content_type.family` as `Cow::Owned`. This
> defeats the `Cow::Borrowed`/zero-alloc invariant documented for well-known families (and used by
> `EffectRequest::new`). Accept `Into<Cow<'static, str>>` and canonicalize recognized families to
> `Cow::Borrowed(kind.family())`, only owning the string for true extension families.

The field is `content_type.family: Cow<'static, str>` (the zero-alloc-for-well-known design from the
#1563 Cow<str> work). `new_with_family` allocating an Arc→String→Cow::Owned for EVERY family — including
`http`/`model`/etc. that have a `&'static str` — regresses that invariant on the new constructor path.
Canonicalize a recognized family to `Cow::Borrowed(kind.family())`, owning only genuine extension family
strings. LOW-MED/perf-contract (matches the #1563 direction). Fix-forward or before-land.
