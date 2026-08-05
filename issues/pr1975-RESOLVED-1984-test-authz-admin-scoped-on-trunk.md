# PR #1975 review — cdz-agent-host/src/async_host.rs (v-agent-harness-host) — MERGED — test-precision [VERIFIED]

https://github.com/camshaft/cadenza/pull/1975 (enforce admin authz in the loop — MY #1967 MED-HIGH fix,
landed). Copilot (id 3710090074) flags the loop tests' authorizer is broader than its doc, weakening them.

## `test_authz` doc says "grants only the `admin` principal" but uses `allow_all_for_local_admin()` (wildcard ANY principal) → loop tests won't catch principal-plumbing bugs (Copilot, async_host.rs:648) — test-precision [VERIFIED]
> `test_authz` is documented as granting only the "admin" principal, but it currently uses
> `AllowList::allow_all_for_local_admin()`, which grants every action to ANY principal via the wildcard
> ("*"). This makes the loop tests less strict (they won't catch principal-plumbing mistakes) and
> contradicts the comment.

VERIFIED on trunk. `test_authz` (async_host.rs:646) doc: "grants the `"admin"` principal every action";
body: `Box::new(AllowList::allow_all_for_local_admin())`. And `allow_all_for_local_admin` (admin.rs:132)
doc'd as "Allow ANY principal to perform EVERY v0 admin action" — it calls `allow_any_principal(action)`
for each action (wildcard). So the loop tests admit ANY principal, not just "admin": a regression that
plumbed the WRONG principal (or dropped it) through `AdminRequest` → `apply_admin_authorized` would still
pass these tests, since every principal is allowed. Contradicts the doc AND undercuts the very
principal-threading the #1967 fix added (the security-relevant part). LOW/test-precision — the authz
DECISION is unit-tested in admin.rs, but the loop's principal PLUMBING is exactly what these tests should
pin. Fix: build `test_authz` from an allowlist scoped to the `"admin"` principal only (not
`allow_any_principal`), so a test that submits a non-admin/absent principal is DENIED — proving the loop
threads the real principal. v-agent-harness-host owns cdz-agent-host/src. (Good to tighten since it guards
the authz seam I flagged in #1967.)
