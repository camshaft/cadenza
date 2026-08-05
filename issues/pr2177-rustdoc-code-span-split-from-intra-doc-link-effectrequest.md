# PR #2177 review — cdz-kernel (v-agent-harness) — OPEN — rustdoc-rendering [VERIFIED, LOW] (batched, folds MY #2166)

https://github.com/camshaft/cadenza/pull/2177 (fold #2166 doc-precision — qualify heap_unmarshal's return
as wasm_host::EffectRequest; the fix-forward for MY #2166). Copilot 2 inline, same finding across 2 files →
batched. The semantic fix (disambiguating the two EffectRequest types) is CORRECT; only the rustdoc markup
renders awkwardly.

## the rustdoc `Vec<`[`wasm_host::EffectRequest`](…)`>` splits a code span from an intra-doc link → renders oddly; phrase as "a `Vec` of [`crate::wasm_host::EffectRequest`]" for clean formatting + link (Copilot, lib.rs:47 & heap_unmarshal.rs:10 & :116) — rustdoc-rendering [VERIFIED, LOW]
> [lib.rs:47] `Vec<`[`wasm_host::EffectRequest`]`>` splits a code span and an intra-doc link, which
> renders oddly … Prefer either a single code span or a `Vec` + link phrasing, and make the kernel type
> link explicit as `crate::effect::EffectRequest`.
> [heap_unmarshal.rs:10] mixes code spans and links, which tends to render awkwardly … phrase as "a `Vec`
> of [`crate::wasm_host::EffectRequest`]".

VERIFIED in the #2177 diff: the fold (correctly, per my #2166) disambiguates the type —
"`Vec<`[`wasm_host::EffectRequest`](crate::wasm_host::EffectRequest)`>` — the WIT-generated
component-boundary type, NOT the kernel's public [`crate::effect::EffectRequest`]" (diff:12-13, and
similarly heap_unmarshal.rs:27, :43). So the SEMANTIC precision my #2166 asked for is delivered (both types
now named + linked). The nit is purely rustdoc MARKUP: `` Vec<`` opens a code span that's immediately
broken by an intra-doc `[link]`, then re-opened with `` `>` `` — rustdoc renders this as a fragmented span
(the `Vec<` and `>` as separate inline-code chunks around a link). LOW/rendering only (the text is correct
+ unambiguous; it just looks broken in generated docs). Fix per Copilot: phrase as "a `Vec` of
[`crate::wasm_host::EffectRequest`]" — link the type name cleanly, drop the `Vec<…>` code-span split. 2
files (lib.rs + heap_unmarshal.rs ×2 sites) → batched. v-agent-harness owns cdz-kernel/src. PR OPEN →
foldable. (One-layer-deeper cosmetic residual on the fold of my #2166 — the disambiguation is right, the
markup is fiddly; a clean phrasing closes it.)
