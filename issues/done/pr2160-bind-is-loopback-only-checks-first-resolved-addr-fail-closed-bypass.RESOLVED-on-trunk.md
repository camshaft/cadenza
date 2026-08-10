# PR #2160 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — OPEN — security/fail-closed [VERIFIED, MED] (on the fix for MY #2155)

https://github.com/camshaft/cadenza/pull/2160 (prometheus scrape posture — reject non-loopback bind unless
allow_non_loopback, concierge (a); the HARDENING fix for MY #2155 posture finding — goes beyond the doc
reword to an actual reject-gate). Copilot 1 inline — the new gate has a multi-address hole.

## `bind_is_loopback` only checks the FIRST address from `ToSocketAddrs`, so a hostname resolving to both loopback + non-loopback IPs can be misclassified as loopback → the `allow_non_loopback` reject-gate is skipped even though `TcpListener::bind` may bind a NON-loopback addr; the doc says fail-closed, so it must require ALL resolved addrs be loopback (Copilot, config.rs:407) — security/fail-closed [VERIFIED, MED]
> `bind_is_loopback` only checks the *first* address returned by `ToSocketAddrs`. For a hostname that
> resolves to multiple IPs (some loopback, some non-loopback), this can misclassify the bind as loopback
> and skip the `allow_non_loopback` gate, even though `TcpListener::bind` may end up binding a non-loopback
> address depending on which resolved address succeeds. Since the doc says this check is fail-closed, it
> should treat the bind as loopback only if **all** resolved addresses are loopback (and at least one
> address exists).

VERIFIED in the #2160 diff: `bind_is_loopback` (diff:87-90) is
  `addr.to_socket_addrs().map(|mut it| it.next().map(|a| a.ip().is_loopback()).unwrap_or(false))...`
— it takes only `it.next()` (the FIRST resolved address). The gate (diff:107-112) is `if
!allow_non_loopback && !bind_is_loopback(bind) => reject`. So for a hostname resolving to BOTH a loopback
and a non-loopback IP where the FIRST enumerated addr is loopback, `bind_is_loopback` returns true → the
reject-gate is SKIPPED → the unauthenticated scrape endpoint is allowed WITHOUT the `allow_non_loopback`
opt-in. But `TcpListener::bind(addr)` binds whichever resolved address it can — potentially the
NON-loopback one — so the endpoint can end up publicly bound despite the gate passing. The doc calls this
gate fail-closed ("a name that fails to resolve is treated as non-loopback (fail-closed)", diff:104), so
first-addr-only VIOLATES the stated contract. MED (security posture — this gate IS the guard my #2155
review prompted; a multi-address name is the exact bypass). Same multi-address class as my #2112 (IPv6
bind first-addr) + #2119 (multi-addr test). Fix per Copilot: treat the bind as loopback only if ALL
resolved addresses are loopback AND at least one exists — e.g.
  `let mut addrs = addr.to_socket_addrs().ok()?.peekable(); addrs.peek().is_some() && addrs.all(|a|
  a.ip().is_loopback())` (return false / non-loopback on resolve-error or empty). v-agent-harness-host owns
cdz-agent-host. PR OPEN → foldable pre-merge. (Owning the chain: my #2155 doc-finding drove this reject-
gate; the gate itself has the one-layer-deeper multi-address hole — the recurring pattern. The reject
approach is a GOOD hardening beyond the doc reword; it just needs all-addrs.)
