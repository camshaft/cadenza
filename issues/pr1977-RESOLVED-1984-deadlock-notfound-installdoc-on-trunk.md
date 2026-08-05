# PR #1977 review — cdz-agent-host daemon + admin_socket (v-agent-harness-host) — MERGED — 3 findings [VERIFIED]

https://github.com/camshaft/cadenza/pull/1977 (cdz-agent-daemon: wire the admin control interface). Copilot
3 inline, all VERIFIED. Two are real availability/correctness bugs (deadlock, NotFound race); one is a
doc-vs-behavior mismatch.

## `tokio::join!(host loop, socket serve)` DEADLOCKS the daemon if the host loop returns `Err` early — socket.serve only exits on ctrl-c (Copilot, cdz_agent_daemon.rs:123) — availability/error-handling [VERIFIED]
> `tokio::join!` here can deadlock the daemon on host-loop failure: `AdminSocket::serve` only returns when
> its `shutdown` oneshot resolves, but `sock_sd_tx` is only triggered by ctrl-c. If
> `host.run_with_wall_clock(..)` returns early with `Err(..)`, the join will wait forever on the
> still-running socket task, and the process won't exit or surface the error.

VERIFIED (cdz_agent_daemon.rs:120): `let (loop_result, ()) = tokio::join!(host.run_with_wall_clock(
loop_sd_rx), socket.serve(admin_channel, sock_sd_rx));`. `socket.serve` returns only when `sock_sd_rx`
fires, and `sock_sd_tx.send(())` is called ONLY in the ctrl-c handler (:111-115). So if the host loop
returns `Err` (a kernel error — the loop's fail-fast path), `join!` still blocks on the socket task
forever: the daemon hangs, the error is never surfaced, no exit. MED/availability. Fix: on host-loop
return, fire `sock_sd_tx` so the socket task shuts down too — e.g. `select!` on the loop future and, when it
completes (Ok or Err), signal `sock_sd_tx` then await the socket task; or restructure so either task
completing tears down the other. Surface `loop_result`'s Err to the process exit code.

## second NotFound race: `symlink_metadata(path)?` in `reclaim_dead_socket_then_bind` aborts a recoverable rebind (Copilot, admin_socket.rs:156) — correctness [VERIFIED, follow-on to my #1971]
> This change fixes `remove_file` returning NotFound, but there's still a NotFound race earlier:
> `symlink_metadata(path)?` can fail with `NotFound` if the socket path is unlinked between the original
> `bind(..)` returning `AddrInUse` and entering this function. In that case the path is already free, so
> returning `NotFound` aborts a rebind that could succeed…

VERIFIED (admin_socket.rs:116): `let meta = std::fs::symlink_metadata(path)?;` — a bare `?`. The
`remove_file`-NotFound fix I flagged (#1971) handled the LATER unlink, but the FIRST filesystem touch is
this `symlink_metadata`, which has the same race: if the path is unlinked between the `bind()`→`AddrInUse`
and this call, `symlink_metadata` returns NotFound and `?` aborts — even though the path is now free and a
plain rebind would succeed. LOW-MED/correctness. Fix: treat `symlink_metadata` NotFound as "path already
gone → rebind" — `match symlink_metadata(path) { Ok(m)=>…type check…, Err(e) if e.kind()==NotFound => { /*
raced free; rebind */ return UnixListener::bind(path); }, Err(e)=>return Err(e) }`. (Composes with the
#1971 fix in slice H — both NotFound points need the same treatment.)

## daemon advertises install-session in docs but builds the host with NO factory → install always errors (Copilot, cdz_agent_daemon.rs:11) — doc-vs-behavior [VERIFIED, LOW]
> The module docs say the admin socket serves "install / list / status / stop", but the daemon builds the
> host with `AsyncAgentHost::new(..)` (no `SessionFactory`), so `install-session` will return a "no session
> factory" error … Either wire a real factory via `AsyncAgentHost::with_factory(..)` or adjust the docs.

VERIFIED — the daemon doc (cdz_agent_daemon.rs:9) lists "install / list / status / stop", but with no
factory wired, install-session returns the "no session factory available" error (the deny path from the
#1949 fix). NOTE: #1979 (open) is literally "ComponentSessionFactory — install-session…" — so the factory
wiring is IN FLIGHT. So this is either resolved by #1979 (then just a transient doc lead) or the doc should
soften "install" to "planned" until #1979 lands. LOW/doc — likely mooted by #1979; flag to confirm.
v-agent-harness-host owns cdz-agent-host/src.
