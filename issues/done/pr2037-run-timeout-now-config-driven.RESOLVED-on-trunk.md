# PR #2037 review — xtask/src/main.rs (v-fleet-tooling) — MERGED — correctness [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2037 (xtask run: bound the compile+run pipeline at run_timeout(),
killing a hung stage). Copilot (id 3713077741) flags `run_timeout()` read twice.

## `run_timeout()` (an env-var read) is called twice — for the enforced timeout AND the error message — so the reported deadline can disagree with the enforced one (Copilot, main.rs:621 & :648) — correctness [VERIFIED]
> `run_timeout()` is read twice (once for the enforced timeout, once for the message). Since it comes from
> an env var, reading it twice can make the reported deadline disagree with the one enforced. Bind it once
> and reuse the same value for both the call and the error message.

VERIFIED on trunk: `wait_stages_with_timeout(…, run_timeout(), …)` (main.rs:599) enforces the timeout, then
the hang message does `run_timeout().as_secs()` (:614) — a SECOND call to `run_timeout()`, which reads the
env var (`run_timeout` defined at :700). If the env var changed between the two reads (or is re-evaluated
non-deterministically), the killed-at deadline and the "…did not finish within {N}s" message disagree —
misleading the operator about the actual bound. LOW/correctness (the window is tiny and env vars rarely
change mid-run, but it's a free consistency fix). Fix per Copilot: bind once —
`let timeout = run_timeout();` before the wait, pass `timeout` to `wait_stages_with_timeout`, and use
`timeout.as_secs()` in the message. (Same pattern applies at the other `wait_with_timeout(child,
run_timeout())` sites at :1111/:1175/:1261/:1326/:1464 IF any of them also reads it again for a message —
worth a glance, though those pass it once each.) v-fleet-tooling owns xtask/src/main.rs.
