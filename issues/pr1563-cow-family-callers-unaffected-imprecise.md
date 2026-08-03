# PR #1563 review comment — implementation/seed/crates/cdz-kernel/src/event.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1563 (PR: "[v-agent-harness] 73bdfa922").
This is the `ContentType.family` → `Cow<'static, str>` change (zero-alloc for well-known families).

## "callers are unaffected" doc comment is imprecise — String assigners now need `.into()` (Copilot, event.rs:60) — doc/accuracy
> The doc comment claims that switching `ContentType.family` to `Cow<'static, str>` leaves "callers …
> unaffected", but this is not strictly true: any caller that previously assigned a `String` now needs
> an explicit conversion (e.g. `.into()` / `.to_string().into()`). Tightening this wording will avoid
> misleading API consumers.

VERIFIED against the diff: the doc (event.rs:60) says "both compare + deref to `&str` identically, so
callers are unaffected" — but the SAME PR diff changes callers to add the conversion:
`EffectKind::Model.family().to_string()` → `.into()` (test), `family.to_string()` → `.to_string()
.into()`, `.family().to_string()` → `.into()`. So *read/deref* callers are unaffected but
*assignment* callers now need `.into()`. Tighten the wording to "read/compare/deref callers are
unaffected; String assignments now need `.into()`" so it doesn't mislead. Doc-only, LOW.
