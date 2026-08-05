# PR #1971 review — cdz-agent-host/src/admin_socket.rs (v-agent-harness-host) — MERGED — correctness [VERIFIED]

https://github.com/camshaft/cadenza/pull/1971 (harden the admin socket — the #1962 fix-forward: bind-first
+ dead-socket reclaim + per-connection spawn). Copilot (id 3710016226) flags a follow-on bug IN that fix:
the NotFound race arm still hits an unconditional `remove_file?`.

## `reclaim_dead_socket_then_bind`: the `NotFound` connect arm falls through to `remove_file(path)?`, which returns `NotFound` → recovery fails even though a plain rebind would succeed (Copilot, admin_socket.rs:154) — correctness [VERIFIED]
> `reclaim_dead_socket_then_bind` intends to treat `connect()` returning `NotFound` as a race ("just
> rebind below"), but it unconditionally calls `remove_file(path)?` afterwards. In the `NotFound` case (or
> if another process removes the path between the checks), `remove_file` will return `NotFound` and the
> recovery path fails even though a plain re-bind would succeed.

VERIFIED on trunk. `reclaim_dead_socket_then_bind` (admin_socket.rs:113) matches on `UnixStream::connect`:
`ConnectionRefused` → "dead socket, safe to reclaim"; `NotFound` → arm comment "Raced away between bind and
here; just rebind below". But AFTER the match it does `std::fs::remove_file(path)?; UnixListener::bind(path)`
unconditionally. In the `NotFound` case the path is already gone, so `remove_file` returns `NotFound` and
`?` propagates it as an `io::Error` — the recovery aborts, even though `UnixListener::bind(path)` would now
succeed (the path is free). Same for a TOCTOU where another process unlinks the path between
`symlink_metadata` (line 115) and here. The `NotFound` arm's stated intent ("just rebind below") is
defeated by the very next line. LOW-MED/correctness — a rare race, but it turns a recoverable state into a
hard bind failure. Fix: ignore `NotFound` from `remove_file` —
`match std::fs::remove_file(path) { Ok(()) => {}, Err(e) if e.kind()==NotFound => {}, Err(e) => return
Err(e) }` then bind. (Or skip `remove_file` entirely in the `NotFound`-connect arm.) v-agent-harness-host
owns cdz-agent-host/src. Fix-forward since #1971 merged. Note: this is the socket-unlink fix from MY #1962
finding — a good fix that introduced one edge-case regression the reviewer caught; worth closing the loop.
