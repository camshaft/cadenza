# PR#1034 — two review comments: kernel.rs token.clone perf (v-agent-harness) + int-width.cdz stale test comment (v-compiler-ml)

Two Copilot review comments, split by owner. blame `de35be1bb` (batch #151-ish: continuation-token §19e +
de-sentinel neg-literal). Gates: kernel.rs = cdz-kernel own `cargo test`+clippy; int-width.cdz =
compiler-ml self-host + `cargo test -p rcdzc`.

## Comment A (verbatim) — kernel.rs:380 (id 3696718515) → v-agent-harness

- "`token.clone()` allocates and copies the continuation token even though `token` is already owned after
  destructuring `effect`. This adds unnecessary overhead on every dispatched effect; you can move the
  token directly into the durable `Dispatched` frame."

### Liaison verification (confirmed on trunk 820f76a0d)

`drive_worklist` (kernel.rs:317-320) destructures the popped effect: `let Effect { request: req, token }
= effect;` — so `token: Option<Vec<u8>>` is OWNED. At :380 the `Dispatched` frame is built with `token:
token.clone()`. `token` is NOT used anywhere after :380 (only `req` is, via `executor.perform(&req, …)` at
~:410 — which is why `req.kind`/`req.target` ARE cloned at :372-373 but `token` need not be). So the clone
is a needless per-dispatch allocation on the (potentially non-empty) continuation token; moving `token`
into the frame (`token,` / `token: token`) is correct and cheaper. Minor perf, correctness-neutral.

Owner: **v-agent-harness** (`cdz-kernel/src/kernel.rs`:380). Move `token` into the `Dispatched` frame
instead of `token.clone()` (it's owned + unused afterward). Micro-opt, per-dispatch.

## Comment B (verbatim) — int-width.cdz:294 (id 3696718526) → v-compiler-ml

- "This comment is now inaccurate after switching to negative literals: the expression being tested is
  `checked-sub(0, 1, ...)` (i.e., `0 - 1 = -1`), not just `-1` on its own. Restoring the full expression
  keeps the test documentation clear."

### Liaison verification (confirmed on trunk 820f76a0d)

`iw-checked-sub-underflow-is-none` (int-width.cdz:293-294): the comment reads "`-1 = -1` underflows UInt8
(min 0) → None; an in-range signed sub computes: 5 - 8 = -3 fits Int8 → Some." but the tested expression
is `checked-sub(0, 1, false, 8)` = `0 - 1 = -1`. The de-sentinel neg-literal switch (this PR) evidently
rewrote the comment's LHS to a bare `-1 = -1` (nonsensical — `-1` isn't the operation), losing the `0 - 1`
operands. Copilot's right: restore `0 - 1 = -1 underflows UInt8 …`. The `5 - 8 = -3` half is already
correct. Doc-only (a test comment); no code/behavior change.

Owner: **v-compiler-ml** (`implementation/compiler-ml/src/int-width.cdz`:294). Restore the comment to
`0 - 1 = -1 underflows UInt8 (min 0) → None; …`. Doc-only, low urgency (compiler-ml de-prioritized).
