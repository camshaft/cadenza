# PR #1736 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1736 (MERGED). The #1722→#1727→#1732→#1736 Cow zero-alloc chain.

## `new_with_family` takes `impl Into<Arc<str>>` + `.into()` → allocates for a `&'static str`, defeating its own canonicalization (Copilot, kernel.rs:399) — perf [VERIFIED]
> `EffectRequest::new_with_family` takes `family: impl Into<Arc<str>>` and immediately does `family.into()`,
> which ALLOCATES when passed a `&'static str` like `effect_ct::CAPABILITIES`. Since `new_with_family` then
> canonicalizes well-known families back to `Cow::Borrowed`, this seed path does extra heap work vs the
> prior `EffectRequest::new(EffectKind::Emit, …) + content_type.family = CAPABILITIES` (alloc-free for the
> family tag). Consider keeping the prior constructor pattern here (or a static-family overload).

The signature `impl Into<Arc<str>>` forces `Arc::from(&'static str)` = a heap alloc BEFORE the
canonicalization can map it back to `Cow::Borrowed` — so the seed path allocates-then-discards for exactly
the well-known families the #1722/#1727 invariant protects. This is the ORIGINAL constructor-signature
choice (upstream of the #1727 body-canonicalization fix), so the alloc happens at the boundary regardless.
Options: (a) change the param to `Into<Cow<'static, str>>` so a `&'static` stays borrowed end-to-end
(matches #1727's intent), or (b) at the seed call-site, use the alloc-free prior pattern. LOW-MED/perf —
the definitive close of the zero-alloc chain (the param type, not just the body). Fix-forward.
