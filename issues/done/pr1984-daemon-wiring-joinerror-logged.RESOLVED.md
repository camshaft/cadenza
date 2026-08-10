# PR #1984 review — cdz-agent-host/src/bin/cdz_agent_daemon.rs (v-agent-harness-host) — OPEN — observability [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1984 (review follow-ups — the daemon loop-Err DEADLOCK fix for
MY #1977 finding, + #1975/#1979). Copilot (id 3710585434) flags the new socket task's JoinHandle result is
discarded.

## `let _ = socket_task.await;` ignores the socket task's `JoinError` → a socket-server PANIC exits the daemon with no indication (Copilot, cdz_agent_daemon.rs:135) — observability [VERIFIED, LOW]
> The admin socket task's JoinHandle result is ignored. If the socket server panics, the daemon will exit
> without any indication of why the socket teardown failed, which makes diagnosing shutdown/exit behavior
> harder.

VERIFIED in the #1984 diff — the deadlock fix (great, resolves my #1977 (A)) replaces `tokio::join!` with:
`let socket_task = tokio::spawn(socket.serve(admin_channel, sock_sd_rx)); let loop_result =
host.run_with_wall_clock(loop_sd_rx).await; let _ = sock_sd_tx.send(()); let _ = socket_task.await;`. The
final `let _ = socket_task.await;` discards the `Result<(), JoinError>` — if `socket.serve` PANICKED, the
`JoinError` is swallowed and the daemon proceeds to its exit-code match on `loop_result` alone, giving no
signal that the socket side died. LOW/observability — doesn't affect the exit code (the loop result drives
that, correctly), but a panicked socket server should at least be logged. Fix: `if let Err(e) =
socket_task.await { eprintln!("cdz-agent-daemon: admin socket task failed: {e:?}"); }`. Minor polish on an
otherwise-correct deadlock fix. v-agent-harness-host owns cdz-agent-host/src. (PR still open → foldable.)
