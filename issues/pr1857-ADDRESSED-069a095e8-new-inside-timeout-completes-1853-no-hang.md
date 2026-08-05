# PR #1857 review comment — cdz-agent-host/tests/live_transport_e2e.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1857 (MERGED — the #1853 live-call timeout fix). Residual on the fix.

## `BedrockModelTransport::new().await` is OUTSIDE the timeout → construction can still hang (Copilot, live_transport_e2e.rs:130) — test-robustness [VERIFIED]
> `LIVE_CALL_TIMEOUT` is described as a hard ceiling on any single live call, but the Bedrock test awaits
> `BedrockModelTransport::new().await` OUTSIDE a timeout. `new` explicitly may probe the environment (IMDS)
> — so it can still hang the opt-in run, contradicting the doc's guarantee. Wrap construction in the same
> timeout (or relax the doc to exclude init).
VERIFIED on trunk: `let transport = BedrockModelTransport::new().await;` (:126) is outside the
tokio::time::timeout, which wraps only `transport.invoke(...)` (:127-132). Since `new()` probes the env
(IMDS/creds resolution — can stall), a hang there escapes the "hard ceiling on any single live call" the
#1853 timeout was added to guarantee. So the #1853 fix left the CONSTRUCTION path uncovered. Fix: wrap
`new().await` in `tokio::time::timeout(LIVE_CALL_TIMEOUT, ...)` too (or narrow the doc to "invoke calls").
LOW-MED/test-robustness — completes the #1853 no-hang guarantee. Fix-forward.
