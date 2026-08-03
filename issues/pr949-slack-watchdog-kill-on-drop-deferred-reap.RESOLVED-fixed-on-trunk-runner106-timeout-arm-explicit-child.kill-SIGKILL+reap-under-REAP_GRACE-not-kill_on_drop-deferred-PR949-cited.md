# PR#949 review comment — slack-bridge watchdog timeout relies on kill_on_drop (deferred reap → orphan buildup) (v-slack-bridge)

Mirrored from GitHub PR#949 review comment (Copilot), id `3691801574`.
File: `fleet/slack-bridge/src/runner.rs:77` — slack-bridge → v-slack-bridge. Blame `8727beef3`
"slack-bridge watchdog: bound each fire with a timeout so one hung child can't stall the daemon's re-arm
loop (operator Track A resilience)".

## Comment (verbatim)

- (id 3691801574, fleet/slack-bridge/src/runner.rs:77) "On timeout, this relies on `kill_on_drop(true)`
  + dropping the `wait_with_output()` future to kill the child. Tokio's `kill_on_drop` cleanup/reaping is
  best-effort and may be deferred to a background reaper; if the watchdog wedges repeatedly, this can
  build up an orphan queue and delay process reaping. Consider explicitly killing and then `wait()`ing
  the child in the timeout path to ensure timely cleanup (and optionally use a short secondary timeout
  for the reap)."

## Liaison verification (confirmed on trunk 16f366838)

The watchdog-fire timeout arm (runner.rs:~73-78): the comment "the child (kill_on_drop) is dropped here →
killed" — the timeout path relies on `Command::kill_on_drop(true)` + dropping the timed-out
`wait_with_output()` future to terminate the child. Copilot's concern is real for Tokio: `kill_on_drop`
sends the kill but the actual REAP (waitpid) is deferred to Tokio's background signal reaper — best-effort,
not synchronous. If the watchdog fire times out REPEATEDLY (a persistently-hung child, exactly the case
this timeout was added to survive), each dropped future leaves a zombie pending reap → an orphan/zombie
QUEUE that delays process reaping and leaks PIDs over time. Fix (Copilot's, sound): in the timeout path,
explicitly `child.kill().await` then `child.wait().await` (optionally under a short secondary timeout) so
the reap is timely and bounded, rather than trusting the background reaper. Resilience — matters precisely
because this is the "persistently-hanging watchdog" path. Owner's call on severity (a slack-bridge daemon
watchdog; low blast radius but a real slow leak under repeated wedge).

Owner: **v-slack-bridge** (`fleet/slack-bridge/src/runner.rs` watchdog, `8727beef3`). Explicit
kill+wait (± secondary timeout) in the timeout path instead of relying on kill_on_drop's deferred reap.
