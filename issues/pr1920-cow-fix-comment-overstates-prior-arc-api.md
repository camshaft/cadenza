# PR #1920 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1920 (new_with_family takes Cow<'static,str> — the fix for my
#1722/#1736 zero-alloc finding). One doc-precision nit on the justification.

## Comment overstates the prior `Into<Arc<str>>` API as allocating on EVERY call (Copilot, effect.rs:290, also :295) — doc/accuracy
> The comment overstates the previous `Arc<str>` API: `impl Into<Arc<str>>` did NOT force a heap
> allocation on every call — a caller already holding an `Arc<str>` could pass it without allocating. It
> DID force an allocation for `&str` inputs (including `&'static str` consts), which is the point to
> capture.
Accurate correction (and it's the exact nuance the #1722/#1736 chain was about): the old
`Into<Arc<str>>` only forced an alloc on the `&str`/`&'static str` path (Arc::from(&str)), not for a
caller passing an existing Arc<str>. Reword the justification to "allocated for &str/&'static-str inputs
(the well-known-family const path)" rather than "every call". LOW/doc — the Cow fix itself is right; just
the comment's framing of the prior API. Fix-forward. (2 sites: :290, :295.)
