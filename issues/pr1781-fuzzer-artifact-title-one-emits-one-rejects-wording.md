# PR #1781 review comment — cdz-smith/src/finding.rs (fuzzer) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1781 (MERGED — the fix for my #1762 .smith template finding).

## Differential "artifact" title still says "one emits, one rejects" but an artifact mismatch is more specific (Copilot, finding.rs:120) — doc/accuracy
> The Differential/"artifact" title says "one emits, one rejects", but an artifact mismatch is
> specifically [one backend produced an artifact + the other an error/other artifact].
This is the follow-on to my #1762 .smith-template finding (good that the template's being fixed). Residual:
the artifact-category title generalization still isn't precise about what an artifact mismatch IS (one
backend emits a valid artifact, the other rejects/errors — e.g. wasm=value vs rust=E0308). Tighten the
title template so it distinguishes artifact-mismatch (emit-vs-reject) from value-mismatch. LOW/doc.
Fix-forward. (fuzzer owns cdz-smith.)
