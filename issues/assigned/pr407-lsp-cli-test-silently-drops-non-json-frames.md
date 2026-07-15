# PR review comments — mirrored from GitHub PR #407 (inline)

- **PR:** #407 "fleet: thirty-second batch (diagnostics, duvet, breaker, cdz-tooling, metaprog, lsp, curried-closure fix)" (MERGED)
- **File:** `implementation/seed/crates/cdz/tests/lsp_cli.rs` (parse_frames @41, JSON-validation @57)
- **Reviewers:** Copilot + amazon-q-developer[bot] (automated)
- **Comment ids:** 3591164547 (copilot), 3591156124 (amazon-q)
- **Links:** https://github.com/camshaft/cadenza/pull/407#discussion_r3591164547 , #discussion_r3591156124

## Comments (verbatim)
> `parse_frames` silently drops any frame whose body isn't valid JSON (`if let Ok(...)`). That can let protocol regressions slip by (the test may still pass while the server emits malformed JSON for some messages). Since this is an end-to-end LSP framing/protocol test, it should fail hard on invalid JSON bodies.
>
> (amazon-q) Missing validation that `lsp_output` contains valid JSON before string matching. If the LSP command fails or produces non-JSON output, the test will pass incorrectly when it should fail.

## Liaison triage — CONFIRMED against trunk
Confirmed in lsp_cli.rs `parse_frames`: `if let Ok(v) = serde_json::from_slice(&data[body_start..body_end])
{ out.push(v); }` — a body that fails to parse is SILENTLY dropped. For an end-to-end LSP
framing/protocol test, a malformed-JSON message would be skipped rather than failing the test, so a
protocol regression (server emits invalid JSON for some message) could pass unnoticed. Both reviewers
flag the same weakness. FIX: make parse_frames fail hard (`.expect`/panic) on an invalid JSON body
within a well-framed message (Content-Length honored) rather than swallowing it. Test-robustness in the
cdz LSP tests; no LSP vertical → route to `corpus-bugfix` PM. Fix on `trunk`. Quotes + links in queue
file.
