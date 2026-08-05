# PR #2169 review — cdz-kernel (v-agent-harness) — OPEN — 1 security/DoS (MED) + 1 doc/dep-floor (LOW) [VERIFIED]

https://github.com/camshaft/cadenza/pull/2169 (tracing facade on the drive loop — kernel emits, never
subscribes). Copilot 2 inline.

## the drive-loop tracing events log guest-controlled `req.target` (+ `%reason` formatted from `req.target` in authz.rs) → secret/PII leak in URLs/paths + log-volume blowup on attacker-sized targets (Copilot, kernel.rs:767 & :906) — security/DoS [VERIFIED, MED]
> These tracing fields log guest-controlled strings (`req.target`) and `%reason` (formatted from
> `req.target` in `authz.rs`). That can leak secrets/PII embedded in URLs/paths and can also blow up log
> volume. Consider logging only non-sensitive metadata (lengths and identifiers) and rely on the durable
> event log for full details if needed.

VERIFIED in the #2169 diff: the new `warn!`/`debug!` events log `target = %req.target` (diff:109, :125)
and `%reason` (diff:110, :138). `req.target` is GUEST-controlled (the effect request's target — a URL /
path / command from the reducer), and per Copilot `%reason` is formatted from `req.target` in authz.rs. So
a target carrying a secret (a URL with a token/query param, a path with PII) is emitted into the tracing
stream, and a large target inflates log volume. Same untrusted-input-in-output class as my #2050 (`{val:?}`
DoS) and #2090 (untrusted field name in error). MED (a no-auth log sink is a lower bar than an error path,
but tracing output routinely ships off-box — v-ah-host owns the subscriber that records/exports it, so
guest secrets would land in the telemetry pipeline). Fix per Copilot: log non-sensitive metadata only —
`target.len()`, an effect id/family, the EffectKind — and leave the full `target` to the durable event log
(EventBody::AuthzDenied already captures `reason`/`token`, diff:114). The kernel-emits-never-subscribes
design is good; it just shouldn't emit raw guest strings into spans. (Note `reason` at diff:138 is the
HOST's classification string per its own comment — that one's fine; it's the `%req.target` / target-derived
`%reason` at the authz denial that leak.)

## Cargo.toml comment claims `tracing` is "ALREADY in the tree transitively via wasmtime" (no new dep floor), but wasmtime depends on `log` NOT `tracing`, and this PR ADDS tracing as a new lock package (Copilot, Cargo.toml:68) — doc/dep-floor [VERIFIED via lock, LOW]
> The comment claims `tracing` is already present transitively via `wasmtime`, but `Cargo.lock` shows
> `wasmtime` depends on `log` (not `tracing`) and `tracing` is introduced by this change.

VERIFIED empirically against implementation/seed/crates/cdz-kernel/Cargo.lock: on TRUNK (pre-#2169) there
are ZERO `name = "tracing"` package entries, and `wasmtime`'s dependency list contains `"log"` but NOT
`"tracing"`. The #2169 diff ADDS `+name = "tracing"` as a new `[[package]]` (+ tracing-attributes,
tracing-core) — proving tracing is INTRODUCED here, not pre-existing. So the Cargo.toml comment (diff:63
"NO new dep floor: `tracing` (+ -core) is ALREADY in the tree transitively via wasmtime") is factually
WRONG. LOW/doc (no functional issue — adding tracing is fine and intended; only the "no new dep floor"
justification is false, and a false dep-floor claim can mislead a future audit of the kernel's minimal
dependency surface). CONFIRMED (not just plausible — the lockfile is dispositive). Fix per Copilot: reword
to acknowledge tracing (+ tracing-core/-attributes) as a NEW, intentional dependency for the observability
facade (wasmtime brings `log`, a different facade). v-agent-harness owns cdz-kernel/src. PR OPEN → both
foldable. The guest-target leak is the one that matters.
