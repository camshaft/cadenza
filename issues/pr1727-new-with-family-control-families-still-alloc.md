# PR #1727 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1727 (new_with_family preserves the Cow zero-alloc invariant —
the fix for my #1722 finding). The EffectKind-family canonicalization is right; Copilot flags control/*
still allocating.

## Well-known `control/*` families (CAPABILITIES/SUMMARY) still hit Cow::Owned — miss the zero-alloc invariant (Copilot, effect.rs:257) — perf/consistency [VERIFIED]
> `new_with_family` only canonicalizes families that map to an `EffectKind`. Well-known `control/*`
> families like `effect_ct::CAPABILITIES`/`SUMMARY` are documented stable constants, but fall into the
> `None` branch and get `Cow::Owned(family.to_string())`, reintroducing a heap alloc even for these
> well-known strings. If the intent is "zero-alloc for well-known families", canonicalize those control
> families to `Cow::Borrowed` too.

VERIFIED in the #1727 diff: canonicalization is `match EffectKind::from_family(&family) { Some(k) =>
Cow::Borrowed(k.family()), None => Cow::Owned(family.to_string()) }`. The control-plane families
(`effect_ct::CAPABILITIES`/`SUMMARY`) don't map to an `EffectKind` (they're control-plane, not
world-effects), so they take the `None` branch → owned alloc — despite being stable `&'static str`
constants (the exact well-known case the #1563/#1722 zero-alloc invariant covers). Add a control-family
canonicalization arm (match the known `control/*` constants → `Cow::Borrowed(effect_ct::CAPABILITIES)`
etc.) before the generic `None`-owns fallback. LOW/perf — completes the #1722→#1727 zero-alloc closure.
Fix-forward.
