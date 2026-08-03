# PR #1782 review comment — cdz-agent-host/src/http.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1782 (MERGED).

## `from_name` doc says kernel returns method names lowercase, but decode returns them as-encoded (Copilot, http.rs:62) — doc/accuracy
> The `from_name` docs say the kernel returns method names lowercase, but `decode_http_request` returns the
> `(method ...)` name as encoded (case preserved).
Reconcile: either the doc's lowercase claim is wrong (decode preserves case) or from_name should
lowercase-normalize. Verify against decode_http_request's actual method-name handling. LOW/doc (or a
case-sensitivity bug if a caller relies on the lowercase claim). Fix-forward.
