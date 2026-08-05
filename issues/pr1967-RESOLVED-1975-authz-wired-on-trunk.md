# PR #1967 review — cdz-agent-host/src/admin.rs (v-agent-harness-host) — MERGED — security-relevant + 2 LOW

https://github.com/camshaft/cadenza/pull/1967 (admin authorization seam — deny-by-default). Copilot 3
inline. The HEADLINE one (id 3709823409) is a VERIFIED "gate exists but isn't in the call path" gap —
security-relevant; the other two are LOW (future-proofing + perf).

## the deny-by-default `apply_admin_authorized` is NOT invoked by the real control path — the host loop still calls the UN-authorized `apply_admin` (Copilot, admin.rs:273) — security/correctness [VERIFIED]
> `apply_admin_authorized` is documented as "the deny-by-default gate the daemon's control interface
> actually calls" … but the current control-path in `AsyncAgentHost` still routes admin requests through
> `host.apply_admin(...)` (see `async_host.rs:314`). As a result, the new deny-by-default authorizer is
> not enforced by the existing admin channel/socket path. … thread a `principal` (from the transport) and
> an `AdminAuthorizer` into the loop, and invoke `apply_admin_authorized` instead of `apply_admin`.

VERIFIED on trunk. `apply_admin_authorized` (admin.rs:274) is the deny-by-default gate, but its ONLY
callers are its own unit tests (admin.rs:665/689). The live control path is `handle_admin` (async_host.rs
:309) → `host.apply_admin(req.command, factory, Some(now_ms)).await` (:314) — the UN-authorized entry
point. So the admin channel + Unix-socket transport (#1962) apply commands with NO authorization; the
new gate is dead code w.r.t. the real path. For an interface that installs/stops sessions, "authorizer
written but not wired" reads as protected when it isn't. MED-HIGH (security-seam): the whole point of a
deny-by-default gate is defeated if the default path bypasses it. Fix per Copilot: thread a `principal`
(from the transport — the socket peer / channel caller) + an `AdminAuthorizer` through `AdminRequest` →
`handle_admin`, and call `apply_admin_authorized` instead of `apply_admin`. (Note this composes with the
#1962 socket findings — the socket is the principal source.) Given it merged, a fix-forward MR.

## `AdminAuthorizer` is sync but its real (Cedar-component) impl will be async → breaking API churn (Copilot, admin.rs:88) — API-future-proofing [VERIFIED, LOW]
> `AdminAuthorizer` is currently a synchronous trait, but the doc says the real deployment swaps in a
> Cedar-policy-component impl "reusing the ComponentAuthorizer path" — that path is async … keeping this
> trait sync likely forces a breaking API change when the component-backed authorizer lands. Consider
> `#[async_trait::async_trait(?Send)]` + `async fn authorize(...)`; then `apply_admin_authorized` awaits.

VERIFIED — the trait's own doc names the async Cedar-component path as the eventual impl, so a sync
signature now is a known future break. Mirroring `cdz_kernel::authz::Authorize` (already `async_trait
(?Send)`) makes the seam async-ready before a consumer depends on the sync shape. LOW/design — cheap to
do while the trait has one impl (`AllowList`) + no external callers. Owner's call on doing it now vs at
component-land.

## `AllowList::authorize` allocates `principal.to_string()` + `action.to_string()` per check (Copilot, admin.rs:155) — efficiency [VERIFIED, LOW]
> `AllowList::authorize` allocates multiple `String`s on every check … for each `HashSet::contains` call.
> … Allocate `principal`/`action` once and reuse them for the membership checks.

VERIFIED — avoidable allocs on the authz check (admin path, so not hot, but trivially cleaner). If the
`HashSet`s are `HashSet<String>`, `contains` can take `&str` via `Borrow`, so the `.to_string()`s may be
droppable entirely (pass `principal`/`action` as `&str`). LOW/efficiency. v-agent-harness-host owns
cdz-agent-host/src.
