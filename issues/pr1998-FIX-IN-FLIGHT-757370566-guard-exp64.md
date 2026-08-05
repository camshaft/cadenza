# PR #1998 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — MERGED — correctness [VERIFIED, NEGLIGIBLE/defensive]

https://github.com/camshaft/cadenza/pull/1998 (RetryConfig backoff — should_retry + backoff_ms). Copilot
(id 3711203537) flags the overflow guard clamps one attempt too early.

## `backoff_ms` guards `exp >= 63 → u64::MAX`, but `exp = 63` (attempt 64) is a VALID shift (`1<<63 = 2^63`); the real UB boundary is `exp >= 64` (Copilot, config.rs:172) — correctness [VERIFIED, negligible impact]
> `backoff_ms` clamps `exp >= 63` to `u64::MAX`, which incorrectly treats `attempt = 64` (exp=63) as
> overflow. That makes the backoff jump to `max_ms` prematurely when `max_ms` is large (e.g. `max_ms =
> u64::MAX`), instead of returning `base_ms * 2^63` which still fits in `u64`.

VERIFIED the math (config.rs:170): `let factor = if exp >= 63 { u64::MAX } else { 1u64 << exp };`. A left
shift is UB/wrap only at `exp >= 64`; `exp = 63` gives `1u64 << 63 = 2^63`, perfectly valid. So the guard
is off by one — `exp == 63` should compute `2^63`, not clamp to `u64::MAX`. Copilot's boundary fix (`exp >=
64`) is correct.

BUT the REACHABLE impact is negligible: `factor` feeds `delay = base_ms.saturating_mul(factor)` then
`.min(max_ms)`. At `exp = 63`: correct `factor = 2^63`; for any `base_ms >= 2`, `base_ms * 2^63` already
saturates to `u64::MAX`, so `.min(max_ms) = max_ms` either way — identical result. The clamp differs ONLY
when `base_ms == 1` AND `max_ms > 2^63` (≈ >292 million years in ms): correct delay `2^63`, code gives
`max_ms`. And attempt=64 (63 doublings) is itself beyond any real `max_attempts`. So it's a
correct-but-unreachable boundary nit (same class as #1929's u128 cast). LOW/defensive.

Fix (safe to apply or dismiss): change the guard to `exp >= 64` so the `exp == 63` case computes `1u64 <<
63` and the saturating-mul + `.min(max_ms)` handle the rest — makes the boundary exactly the UB boundary,
no behavior change in any reachable config. v-agent-harness-host owns cdz-agent-host/src. Owner's call given
the negligible reach.
