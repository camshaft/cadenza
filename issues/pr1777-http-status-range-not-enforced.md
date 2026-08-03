# PR #1777 review comment — cdz-kernel/src/event_ast.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1777 (HTTP codec — request headers + response).

## Doc claims status is 100–599 but the codec enforces no range (Copilot, event_ast.rs:240) — doc-vs-code
> The doc comment claims `status` is constrained to 100–599, but neither `encode_http_response` nor
> `decode_http_response` enforces that range (any `u16` round-trips). Either validate the range in code or
> adjust the docs.

Doc-vs-code mismatch: the codec doc promises a 100–599 status invariant it doesn't enforce (any u16
round-trips). Either validate on encode/decode (reject out-of-range → a decode error) or soften the doc to
"conventionally 100–599; not enforced by the codec". Prefer validation if downstream relies on the range;
else doc. LOW-MED (a stated guarantee the code doesn't provide → a consumer could trust it). Fix-forward.
