# PR #1778 review comments — cdz-kernel/src/event_ast.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1778 (MERGED — HTTP codec headers; "additive-for-callers").

## 1. `decode_http_request` return type changed (method,body)→(method,headers,body) — BREAKING under an "additive" title (Copilot, event_ast.rs:203) — compat/scope [VERIFIED]
> The PR emphasizes "additive-for-callers", but `decode_http_request` changes its public return type from
> `(method, body)` to `(method, headers, body)` — a breaking change for any downstream crate using
> cdz-kernel as a library. Keep the old signature + add `decode_http_request_with_headers`.
VERIFIED on trunk: `decode_http_request` now returns `(String, Vec<(String,String)>, Option<Vec<u8>>)` —
a 3-tuple with headers, vs the prior 2-tuple. The ENCODER side IS genuinely additive (encode_http_request
kept 2-arg, delegates to _with_headers), but the DECODER RETURN is a breaking tuple-shape change — so
"additive-for-callers" holds for encoders, NOT for anyone matching the decode result. Additive-preserving
fix: keep `decode_http_request` returning `(method, body)` (ignoring/hiding headers) + add
`decode_http_request_with_headers` for the 3-tuple, OR retitle to disclose the decode breaking change.
LOW-MED/compat. Fix-forward. (4th harness scope/compat-vs-title instance — #1747/#1768/#1774/#1778 —
escalating the PATTERN to concierge.)

## 2. `status` documented "100–599" but any u16 accepted (Copilot, event_ast.rs:244) — doc-vs-code [dup of #1777]
Same as the #1777 finding (status range unenforced). Covered there. LOW.

## 3. legacy 2-field decode-form doc (Copilot, event_ast.rs:199) — doc
The doc explains the legacy 2-field form decodes "so a payload from the no-headers encoder still decodes" —
verify wording matches the [m,b]→(method, None, body) arm. LOW/doc.
