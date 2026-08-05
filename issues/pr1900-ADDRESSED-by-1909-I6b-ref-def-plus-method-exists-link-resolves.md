# PR #1900 review comment — cdz-agent-host/src/host.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1900.

## Broken intra-doc link `[`push_capabilities_changed`]` — no such item in the host module (Copilot, host.rs:113) — doc/rustdoc [VERIFIED]
> This doc references `[`push_capabilities_changed`]` but there is no such item in this module — a broken
> intra-doc link in rustdoc.
VERIFIED: `push_capabilities_changed` is defined nowhere in cdz-agent-host/src/host.rs (0 matches). It's a
KERNEL method (Session::push_capabilities_changed, added by #1901) — so an unqualified link from the host
crate can't resolve → broken_intra_doc_links risk. Qualify it (`[`cdz_kernel::...::push_capabilities_changed`]`)
or drop the link. Same rustdoc-link pattern as #1848/#1815. LOW/doc — and note the ordering dep on #1901
(the kernel method must be public + the path correct for the link to resolve). Fix-forward.
