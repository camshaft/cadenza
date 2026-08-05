# PR #1962 review — cdz-agent-host/src/admin_socket.rs (v-agent-harness-host) — MERGED — robustness/security [VERIFIED ×2]

https://github.com/camshaft/cadenza/pull/1962 (admin control interface Unix-socket transport). Copilot 2
inline, both VERIFIED. NOTE: the code has comments DEFENDING both choices, so these are design pushbacks on
conscious decisions — but both concerns are real for the adversarial/misconfig case the comments don't cover.

## `bind` unconditionally `remove_file`s the path before binding → can delete an unrelated regular file OR unlink an ACTIVE socket of another running daemon (Copilot, admin_socket.rs:49 & :98) — robustness/safety [VERIFIED]
> `bind` unconditionally unlinks any pre-existing path before binding. This can (1) delete an unrelated
> regular file if the path is misconfigured, and (2) steal/unlink an *active* socket from another running
> daemon (second daemon wins, first becomes unreachable). Prefer attempting `UnixListener::bind` first; on
> `AddrInUse`, only unlink if the existing path is a Unix socket and no listener is accepting connections.

VERIFIED on trunk: `bind` does `std::fs::remove_file(&path)` (ignoring only NotFound) THEN
`UnixListener::bind(&path)`. The doc comment defends it ("a stale file to block bind… a misconfiguration,
not something to defend by leaving a stale file"). But `remove_file` is indiscriminate: it deletes ANY
inode at that path — a regular file (a fat-fingered path config silently destroys data) or an ACTIVE socket
another daemon is currently accepting on (the second daemon unlinks it, binds its own, and the first daemon
is now serving a socket with no name — unreachable, no error). The "stale file" rationale only holds for a
DEAD socket left by a crashed prior instance. Safer (Copilot's): `bind` first; on `AddrInUse`, stat the
path — unlink+rebind ONLY if it's a socket with no live listener (e.g. a connect() probe that refuses).
MED for a multi-daemon or shared-path deployment; LOW if the path is always process-private. v0 auth is
already owner-only 0o600, so not a privilege issue — a liveness/data-safety one.

## serving each accepted connection INLINE lets one slow/stalled client block the accept loop → local DoS (Copilot, admin_socket.rs:87) — availability [VERIFIED]
> Serving each accepted connection inline means one slow or stalled client can block the accept loop,
> preventing other admins from connecting (local DoS). Consider spawning a per-connection task so the
> listener can keep accepting while commands are still serialized through the host loop via `AdminChannel`.

VERIFIED: `serve` awaits `serve_connection(stream, &admin).await` INLINE inside the accept `select!`, and
`serve_connection` loops reading frames until EOF (it supports pipelined commands). So a single client that
connects and then stalls (never sends a full frame, never closes) holds the accept loop indefinitely — no
other admin can connect. The defending comment ("admin traffic is low-volume + serialized through the one
host loop anyway, so no win in spawning") addresses THROUGHPUT but not a STALLED/malicious client: the
serialization point is `AdminChannel` (the host loop), not the accept loop, so a per-connection
`tokio::spawn` keeps `accept()` responsive while commands STILL serialize through `AdminChannel` — no
ordering change, just decoupling a stuck reader from the listener. Fix per Copilot: spawn per-connection
(the connection future needs no extra Send bound in a current-thread runtime; check the runtime flavor).
MED/availability — a local admin DoS. v-agent-harness-host owns cdz-agent-host/src.
