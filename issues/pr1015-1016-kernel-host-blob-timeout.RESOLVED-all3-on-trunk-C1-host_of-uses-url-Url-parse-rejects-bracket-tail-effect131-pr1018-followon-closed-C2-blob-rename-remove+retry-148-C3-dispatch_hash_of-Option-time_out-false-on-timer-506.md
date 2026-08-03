# PR#1015/#1016 — cdz-kernel: host_of allow-list bypass + blob rename-over-existing + time_out_effect panics on timers (v-agent-harness)

Three Copilot review comments, all `cdz-kernel` → v-agent-harness. Gate = cdz-kernel own `cargo
test`+clippy, NOT `xtask check`.

## Comment 1 (verbatim) — effect.rs:134 (PR#1015, id 3696068746) ⚠ ALLOW-LIST BYPASS
blame `03ca289b8` "SEC-F1 host matching — IPv6 literals + …".

- "`host_of` accepts any bracketed authority and ignores whatever comes after the closing `]`. This means
  an invalid/ambiguous target like `http://[::1]evil.com/` would parse as host `::1` and could
  incorrectly satisfy an allow-list entry for `::1`. For a fail-closed parser, the IPv6-literal branch
  should reject any non-empty tail after `]` unless it begins with `:` (port)."

### Liaison verification (confirmed on trunk 5d27fa2af)

`host_of` (effect.rs:130-134): IPv6 branch `rest.strip_prefix('[')… rest.split_once(']')?.0` — takes the
host as everything up to `]` and DISCARDS the tail. So `[::1]evil.com` → host `::1`. In SEC-F1 authz, a
grant `HostIn(["::1"])` would then AUTHORIZE a request to `http://[::1]evil.com/` — an allow-list bypass
(the real host is `evil.com`-ish / ambiguous, but it matches `::1`). This is the SEC-F1 capability gate
(same security surface as the PR#992 injection lane) → fail-closed matters. Fix (Copilot's, sound): after
`]`, the ONLY valid tail is empty or `:port` — reject any other non-empty tail as an invalid target
(return None → deny). Security / fail-closed.

## Comment 2 (verbatim) — blob.rs:142 (PR#1015, id 3696068753) — rename-over-existing (Windows)
blame `3e593dd3f` "DiskBlobStore put no longer trusts a corrupt existing blob + unique temp names
(PR#1010/1011)" — the fix for the liaison's PR#1011 blob route; this is a residual on it.

- "`DiskBlobStore::put` now rewrites a corrupt existing blob, but the `rename(tmp, path)` happens while
  `path` still exists. On platforms where `rename` cannot replace an existing destination (notably
  Windows), this will fail and the corrupt blob will not be healed (and the function returns an error).
  Consider removing the destination and retrying the rename once when the initial rename fails due to an
  existing target, while still ensuring temp cleanup on failure paths."

### Liaison verification (confirmed on trunk 5d27fa2af)

blob.rs:139-142: `if let Err(e) = std::fs::rename(&tmp, &path) { remove_file(&tmp); return Err(e); }`. On
POSIX rename atomically REPLACES an existing `path` (so a corrupt-blob rewrite heals). On WINDOWS
`std::fs::rename` FAILS if `path` exists → the corrupt blob stays UNHEALED and `put` errors. This is the
SAME rename-over-existing class as PR#903 (provider cache) + PR#929 (inbox). Fix: on rename failure due to
existing dest, `remove_file(&path)` + retry the rename once (still clean up tmp on any failure). (SEVERITY:
lower — Linux is the CI/primary platform where POSIX-replace makes it a non-issue; real for Windows.)

## Comment 3 (verbatim) — kernel.rs:392 (PR#1016, id 3696098838) ⚠ PANIC on a timer id
blame `4e034eda6` "Session::time_out_effect — the missing 'or time out' half of the S4 recovery contract".

- "`time_out_effect` treats any id present in `self.open` as timeout-eligible, but `open` is also used
  for timer obligations (`TimerArmed` inserts into `open`). In that case `dispatch_hash_of` will not find
  a `Dispatched` event and will panic, contradicting the docstring claim that 'never dispatched' ids
  return `false`. Consider making `dispatch_hash_of` return `Option<Hash>` and returning `false` from
  `time_out_effect` when there is no `Dispatched` event (timers should be handled via `fire_due_timers`)."

### Liaison verification (confirmed on trunk 5d27fa2af)

`self.open` holds BOTH dispatched-effect obligations AND timer obligations (`TimerArmed` inserts into it).
`time_out_effect` treats any `open` id as timeout-eligible and calls `dispatch_hash_of`, which (per the
comment) panics when no `Dispatched` event exists — which is exactly a `TimerArmed` id. So timing out a
TIMER id PANICS the kernel, contradicting the docstring ("never dispatched → false"). Fix (Copilot's,
sound): `dispatch_hash_of` → `Option<Hash>`, and `time_out_effect` returns `false` on `None` (a timer is
not a dispatched effect; timers fire via `fire_due_timers`). Correctness / panic-avoidance.

Owner: **v-agent-harness** (`cdz-kernel`). effect.rs = SEC-F1 fail-closed host parse (security); blob.rs =
rename-over-existing (Windows, lower-sev); kernel.rs = panic on timer-id timeout (correctness).
