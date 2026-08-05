# PR #2084 review — cdz-agent-host (v-agent-harness-host) — MERGED — 1 MED reproducibility + 3 LOW [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2084 (metrics onto the s2n-quic-dc-metrics registry). Copilot 4
inline.

## `s2n-quic-dc-metrics` dep is on a MOVING `branch = "main"` → non-reproducible builds, breaks seed toolchain when upstream HEAD moves (Copilot, Cargo.toml:101) — reproducibility [VERIFIED, MED]
> The dependency is pulled from a moving `branch = "main"`, which makes builds non-reproducible and can
> break the seed toolchain when upstream HEAD changes. Prefer pinning to a specific `rev` (the lockfile
> already records one).
VERIFIED: `s2n-quic-dc-metrics = { git = "https://github.com/camshaft/s2n-quic", branch = "main" }`
(Cargo.toml:126). The lock pins `rev … #7ec9f027424badd576436d29d05f75c8d8594133` (lock:97), but the
MANIFEST's `branch = "main"` means a `cargo update` (or any lockless resolve) follows HEAD → the seed build
silently drifts + can break when upstream `main` moves. MED. Fix per Copilot: pin `rev =
"7ec9f027424badd576436d29d05f75c8d8594133"` in the manifest (matching the lock), so the dep is
reproducible without relying on the lock alone. (Same reproducibility class as the earlier --locked/aws-sdk
pinning findings.)

## two `started.elapsed().as_micros() as u64` lossy u128→u64 casts (Copilot, host.rs:451 & factory.rs:90) — defensive [VERIFIED, LOW]
> `started.elapsed().as_micros() as u64` is a lossy cast from `u128` to `u64` that will silently truncate
> on overflow. It's safer to saturate (or fall back) before recording.
VERIFIED (both sites). Practically unreachable (u64::MAX µs ≈ 584,000 years of a single latency sample), so
defensive-only — same class as the #1929 now_ms nit. LOW. Fix: `u64::try_from(elapsed.as_micros())
.unwrap_or(u64::MAX)` if wanted; safe to leave. Two identical sites → one helper.

## doc says AgentHost has "no `Default`" but `impl Default for AgentHost` exists (Copilot, host.rs:317) — doc-accuracy [VERIFIED, LOW]
> The doc comment says AgentHost has "no `Default`", but `impl Default for AgentHost` is present below
> (delegating to `new()`). This is contradictory for readers and rustdoc.
VERIFIED — contradiction. LOW. Fix: drop the "no Default" claim (or the impl, if Default isn't wanted —
but delegating to `new()` is fine, so just fix the doc). v-agent-harness-host owns cdz-agent-host/src. The
branch-dep is the one that matters.
