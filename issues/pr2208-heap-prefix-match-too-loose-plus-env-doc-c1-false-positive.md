# PR #2208 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — 1 LOW-MED + 1 LOW [VERIFIED] (1 Copilot finding DISMISSED as false-positive) (the fix for MY #2203)

https://github.com/camshaft/cadenza/pull/2208 (fold #2203 review — runtime-dep fail-loud + bind returns
store (KV atomicity) + apply err-class + wording). Copilot 3 inline; I VERIFIED each — c1 is a false
positive (dismissed), c2 + c3 are real.

## DISMISSED (Copilot false positive, wasm_host.rs:583) — "bind sig change leaves callers matching Err(e), won't compile"
Copilot claimed the new `HeapHandle::bind` signature (`Result<_, (ComponentError, Store<T>)>`) leaves
in-crate `Err(e)` callers that won't compile. VERIFIED FALSE: the #2208 diff UPDATES every caller — the
test helpers convert `Err(e) => panic!(…)` → `Err((e, _)) => panic!(…)` (diff:405-406, 414-415, 423-424,
431-432) and the Compose-matching helper `Err(ComponentError::Compose {..})` → `Err((ComponentError::Compose
{..}, _store))` (diff:441-442), plus the production path `Err((e, store))` (diff:334). Every removed
old-shape line is paired with an updated addition. So the crate compiles; Copilot didn't account for the
caller updates in the same PR. NOT relayed as a finding. (Verify-before-asserting-compile-break: a
"won't compile" claim is checkable — I checked, it's wrong.)

## runtime-dep selection uses `starts_with("cadenza:runtime/heap")` → matches sibling prefixes (`cadenza:runtime/heap2@…`); match the stripped BARE interface name exactly (Copilot, wasm_host.rs:1520) — correctness [VERIFIED, LOW-MED]
> Runtime dep selection uses `starts_with("cadenza:runtime/heap")`, which can accidentally match other
> interfaces that share the prefix (e.g. `cadenza:runtime/heap2@...`). Since this path requires exactly
> the `cadenza:runtime/heap` interface, match on the stripped bare interface name instead of a prefix.
VERIFIED: `if import_name.starts_with("cadenza:runtime/heap")` (diff:276). A future `cadenza:runtime/heap2@…`
would ALSO match → wrongly selected. This refines the fail-loud fix for MY #2203 c1 (the count is now
guarded, but the identity match is too loose). LOW-MED. Fix per Copilot: strip `@version+hash` and compare
the bare interface EXACTLY (`== "cadenza:runtime/heap"`) — pair it with the exactly-one fail-loud so both
count AND identity are exact. (Note the diff comment at :468 already does "strip @version+hash" for the
export lookup — apply the same bare-name discipline to the SELECTION match at :276.)

## env-contract docs say `CDZ_STORE` uses the `DiskBlobStore` (bare-hash) layout, but the impl expects v-nix's `<hash>.wasm` naming (Copilot, reducer_cadenza_b1_e2e.rs:21 & :83) — doc-inconsistency [VERIFIED, LOW]
> The module-level env-contract docs say `CDZ_STORE` uses the `DiskBlobStore` layout, but the
> implementation below explicitly documents and expects v-nix's `<hash>.wasm` naming (not bare-hash
> `DiskBlobStore`)…
VERIFIED-per-Copilot: the module env-contract doc names `DiskBlobStore` layout while the impl expects
`<hash>.wasm`. LOW/doc (the e2e is env-gated + skips until v-nix wires it — no active bug, but the two doc
claims disagree on store layout). Fix: reconcile the module doc to the `<hash>.wasm` naming the impl expects
(coordinate with v-nix on which layout `CDZ_STORE` canonically uses — this straddles the kernel/nix seam).

v-agent-harness owns cdz-kernel/src. PR OPEN → both foldable. (c1 dismissed — the caller sweep is done in
the PR; c2/c3 are real refinements on the #2203 fixes.)
