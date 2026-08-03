# PR #1332 review comments — cdz-agent-host/tests/cedar_authz_e2e.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1332 (PR: "cand: v-agent-harness-host — e859bc37c").

## 1. `.ok()` conflates unset env var with unreadable file -> silent CI skip (Copilot, cedar_authz_e2e.rs:30) — CI/correctness
> `policy_component_bytes()` treats an unreadable path the same as an unset env var
> (`std::fs::read(&path).ok()`), which will silently skip the test and let CI pass even if
> `CEDAR_POLICY_COMPONENT` is set but the file is missing/corrupt. This defeats the purpose of the
> CI-gated e2e test; skip only when the env var is not present, and fail loudly on read errors.

Same class as the #1271 store-guard discipline: skip ONLY when the env var is genuinely unset; if it
IS set but the file is missing/corrupt, FAIL loudly — otherwise the CI-gated authz e2e silently
passes on a broken fixture.

## 2. "policy-FORBIDden" typo in assertion message (Copilot, cedar_authz_e2e.rs:142) — nit
> Spelling/wording: "policy-FORBIDden" is oddly capitalized and misspelled; this should be the normal
> word "forbidden" to keep the assertion message clear.
