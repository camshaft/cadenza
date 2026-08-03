# PR #1831 review comment — cdz-agent-host/src/http.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1831 (real HTTP client transport behind live-net).

## reqwest follows redirects by default → an authorized URL can 3xx to a DISALLOWED host, bypassing the SSRF/host-authz guard (Copilot, http.rs:231) — SECURITY [VERIFIED]
> Reqwest follows redirects by default, which can change the effective destination host. That bypasses
> the "kernel already gated the resolved URL's host" SSRF/exfil guard — an allowed URL could 3xx to a
> disallowed host and still be fetched. Configure the client to NOT follow redirects (or handle redirects
> manually with re-authorization).
VERIFIED against the diff: the client is `reqwest::Client::builder().build()` (http.rs:56-57) with NO
`.redirect(...)` policy — so reqwest's DEFAULT applies (follows up to 10 redirects). The kernel authorizes
the RESOLVED URL's host, but a 3xx response can send the actual fetch to a DIFFERENT (disallowed) host
AFTER authorization → SSRF / data-exfil bypass of the host-authz guard. MED-HIGH (a real security surface
on a live network transport). Fix: `.redirect(reqwest::redirect::Policy::none())` on the builder (or a
custom policy that re-authorizes each hop's host through the kernel). RECOMMEND v-agent-harness-host treat
as a security must-fix before live-net ships broadly.
