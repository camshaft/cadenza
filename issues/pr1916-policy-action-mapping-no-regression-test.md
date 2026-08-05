# PR #1916 review comments — cdz-kernel/src/{name_store,wasm_host}.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1916 (MERGED).

## 1. Correctness-critical policy-ACTION mapping (family, not kind) has NO regression test (Copilot, wasm_host.rs:1088) — test-coverage
> This is a correctness-critical mapping (policy ACTION now comes from `req.content_type.family` instead
> of `req.kind`). No regression test would fail if it reverts (ComponentAuthorizer tests only cover
> construction). Add a test exercising the mapping with a `store/*` request (kind=Emit placeholder,
> family="store/set") to ensure the policy sees "store/set", not "emit".
A silent revert of this mapping (back to req.kind) would send the policy the wrong ACTION (a store/set
authorized as "emit") — a security-relevant misroute — with no test to catch it. Add the store/* mapping
unit test Copilot describes. MED/test-coverage (a correctness-critical + security-adjacent mapping should
be pinned). Fix-forward.

## 2. POLICY_CURRENT doc describes host behavior (Copilot, name_store.rs:127) — doc
The POLICY_CURRENT constant's doc describes host resolve/fetch/rebuild behavior; keep the constant's doc to
what the constant IS + point at the host behavior rather than restating it. LOW/doc.
