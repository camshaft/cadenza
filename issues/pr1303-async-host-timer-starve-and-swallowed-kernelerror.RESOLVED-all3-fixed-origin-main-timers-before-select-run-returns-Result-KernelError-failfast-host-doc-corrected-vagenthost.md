# PR #1303 review comments — cdz-agent-host/src/async_host.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1303 (PR: "cand: v-agent-harness-host — 26899709e").

## 1. Timer starvation under sustained inbound traffic (Copilot, async_host.rs:108) — correctness
> The event loop can starve due timers under sustained inbound traffic: if `rx.recv()` keeps being
> ready, the `sleep` arm may not get polled promptly, so `fire_due_timers` can be delayed arbitrarily
> past the deadline. Consider firing due timers immediately when `next_deadline <= now_ms()` before
> entering `select!`, so timer deadlines are respected even when the inbox is busy.

Real fairness bug in a `select!` loop: a hot inbox can indefinitely defer the timer arm, so timers
fire arbitrarily late. Check `next_deadline <= now_ms()` and fire due timers BEFORE the `select!`
each iteration so deadlines hold under load.

## 2. `deliver` KernelError silently discarded (Copilot, async_host.rs:120) — correctness/robustness
> `AgentHost::deliver` can return `Some(Err(KernelError))`, but the async loop currently discards the
> result (`let _ = ...`). Since `KernelError` indicates log corruption/programming error (not a
> recoverable effect outcome), this should not be silently swallowed; at minimum, fail fast so the
> issue is visible to the operator/supervisor.

A `KernelError` is a corruption/programming-error signal, not a recoverable outcome — `let _ =` hides
it. Fail fast (propagate / abort the loop with the error surfaced) so an operator sees it instead of
a silently-wedged host.

## 3. `host()` accessor doc vs `run(self)` consuming self (Copilot, async_host.rs:69) — doc/API
> The `host()` accessor doc says it's for "a status query over a running host", but `run(self, ...)`
> consumes `self`, so callers cannot hold `&self` and run the loop concurrently. Either adjust the
> wording to "before/after run", or change the API so status queries during `run()` go through the
> event loop.

Since `run(self)` consumes `self`, a concurrent status query via `host()` isn't possible — reword the
doc to "before/after run" (or, if live status is wanted, route it through the event loop). Points 1+2
are the substantive ones; 3 is doc/API-shape.
