# PR#990 review comments — cdz-kernel: AuthzDenied not folded + reducer_hash masks missing-Genesis + u64→usize truncation + corruption-looks-like-EOF (⚠ v-agent-harness)

Mirrored from GitHub PR#990 review comments (Copilot), ids `3695308274` (kernel.rs:230, +232/+258),
`3695308280` (kernel.rs:103), `3695308286` (event.rs:216, +306), `3695308300` (log_store.rs:137).
`implementation/seed/crates/cdz-kernel/*` → v-agent-harness. Blame `e76d6d746` "feat(cdz-kernel): v0.1
kernel spine — durable-dispatch fold loop".

⚠ Several CORRECTNESS/robustness — flagged for v-agent-harness (they own cdz-kernel; NOTE cdz-kernel is
NOT in `xtask check` — its own `cargo test`+clippy is the gate, per [[fmt-check-the-exact-commit-not-just-the-working-tree]]).

## Comments (verbatim)

- (id 3695308274, kernel.rs:230, +232/+258) ⚠ REPLAY DIVERGENCE: "`drive` appends an `AuthzDenied` event
  but never folds it into `kv`. That makes live state diverge from `replay` (which folds every
  non-Genesis event) and prevents reducers from observing denials in real time."
- (id 3695308280, kernel.rs:103): "`reducer_hash` claims it panics on a missing Genesis, but it currently
  returns `Hash::of(b\"\")`, which can silently mask log corruption and produce misleading snapshots."
- (id 3695308286, event.rs:216, +306) ⚠ UNTRUSTED-INPUT TRUNCATION: "Decoding a length-prefixed string
  casts `u64` to `usize` with `as`, which truncates on 32-bit targets. Since the length is untrusted
  input (durable log), this should be bounds-checked and fail with `BadLength` rather than silently
  truncating."
- (id 3695308300, log_store.rs:137): "`decode_frames` treats a complete-but-invalid frame as 'corruption'
  in comments, but it returns `Recovered { torn_tail: false }`, which is indistinguishable from a clean
  EOF. That makes genuine corruption silently look like a clean log end to callers."

### Liaison verification (all plausible on trunk a2875840b; blame `e76d6d746`)

These are on the v0.1 kernel spine (durable-dispatch fold loop). The four claims:
1. **kernel.rs:230 replay-divergence** (⚠): if `drive` (live path) appends `AuthzDenied` to the log but
   does NOT fold it into `kv`, while `replay` folds EVERY non-Genesis event, then a live kernel and a
   replayed-from-log kernel reach DIFFERENT `kv` — a determinism/replay-equivalence break (the core
   invariant of an event-sourced kernel). Highest-severity of the four. (+ :232/:258 same pattern.)
2. **kernel.rs:103 reducer_hash**: doc says "panics on missing Genesis" but returns `Hash::of(b"")` — a
   corrupt/Genesis-less log yields a bogus-but-valid hash instead of failing loudly → misleading
   snapshots. Make it actually panic/err on missing Genesis (match the doc).
3. **event.rs:216 u64→usize `as` truncation** (⚠): a length-prefix from the DURABLE LOG (untrusted) cast
   `u64 as usize` truncates on 32-bit → a huge length wraps small, mis-parsing the frame. Bounds-check +
   fail `BadLength`. (+ :306 same.)
4. **log_store.rs:137 corruption≡EOF**: a complete-but-invalid frame returns `Recovered{torn_tail:false}`
   — indistinguishable from a clean EOF, so genuine corruption reads as a clean log end (silent data
   loss / no alarm). Distinguish corruption from clean-EOF in the return.

All owner's-domain calls on severity/reachability (cdz-kernel is v0.1, may be pre-production), but 1 & 3
are real correctness (replay determinism; untrusted-input truncation) worth prioritizing; 2 & 4 are
fail-loud/observability hardening.

Owner: **v-agent-harness** (`implementation/seed/crates/cdz-kernel/*`; `e76d6d746`). Fold AuthzDenied into
kv (replay-equivalence); make reducer_hash fail on missing Genesis; bounds-check the u64 length →
BadLength; distinguish corruption from clean EOF. Gate = cdz-kernel's own `cargo test`+clippy (NOT `xtask
check`).
