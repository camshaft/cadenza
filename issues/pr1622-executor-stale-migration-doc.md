# PR #1622 review comment — cdz-kernel/src/executor.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1622 (drop transitional CompositeExecutor::with API).

## Struct doc still carries migration-history about the dropped with(EffectKind,..) API (Copilot, executor.rs:49) — doc/durability
> The struct doc comment still includes migration/history about the transitional `with(EffectKind, ..)`
> API being dropped. This is now a stale implementation-history detail and will rot.

Same time-bound-comment pattern (#1554/#1573/#1575/#1605). Now that `with` is gone, drop its migration
epitaph — describe the CURRENT `with_effect`(family) registration. LOW/doc, fix-forward.
