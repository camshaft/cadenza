# PR #1922 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1922 (titled "docs(cdz-kernel): fix 3 broken intra-doc links —
qualify"). This IS the fix for my #1848/#1900 rustdoc-link findings — but it also bundles a public-API change.

## Scope-creep: a "fix broken intra-doc links" PR also changes `new_with_family`'s public signature (Into<Arc<str>>→Into<Cow<'static,str>>) — AND duplicates #1920 (Copilot, effect.rs:283) — process/correctness [VERIFIED]
> The PR is metadata'd as a docs-only fix for broken rustdoc links, but this hunk changes the public
> `EffectRequest::new_with_family` API from `Into<Arc<str>>` to `Into<Cow<'static, str>>` and updates
> semantics/tests. That's a functional/API change (breaks callers passing Arc<str>) — split it out or
> retitle, and consider compat/semver.
VERIFIED against the diff: #1922 changes `new_with_family(family: impl Into<Arc<str>>)` →
`impl Into<Cow<'static, str>>` (effect.rs:8-10) — a public-API change under a "fix broken doc links" title.
TWO problems: (1) scope-creep (docs title carrying an API change — the 5th such harness instance, per the
#1747/#1768/#1774/#1778 pattern I escalated to concierge; the threshold was "5th+ → flag directly"); (2)
this is the SAME Cow change as the dedicated PR #1920 (new_with_family → Cow) — so #1922 and #1920 DUPLICATE
+ will CONFLICT (both edit new_with_family's signature). Recommend v-agent-harness: keep the Cow change in
#1920 (its proper home), strip it from #1922 so #1922 is genuinely docs-only (the 3 link fixes for
#1848/#1900), and avoid the two PRs racing/conflicting on the same signature. MED/process + conflict-risk.
