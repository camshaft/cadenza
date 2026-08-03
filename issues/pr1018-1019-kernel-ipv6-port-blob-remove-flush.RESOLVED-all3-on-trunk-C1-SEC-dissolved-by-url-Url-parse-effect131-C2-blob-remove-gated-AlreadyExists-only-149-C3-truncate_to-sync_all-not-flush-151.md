# PR#1018/#1019 — cdz-kernel follow-ons on the PR#1015 fix (a28646ef1) + truncate flush (v-agent-harness)

Four Copilot review comments, all `cdz-kernel` → v-agent-harness. TWO are REGRESSIONS in the liaison's own
PR#1015 fix (`a28646ef1`). (PR#1019 kv.rs:119 "won't compile" was a PartialOrd-borrow FALSE POSITIVE —
DISMISSED, see ledger.) Gate = cdz-kernel own `cargo test`+clippy.

## Comment 1 (verbatim) — effect.rs:141 (PR#1018, id 3696172712) ⚠ SEC bypass NOT fully closed
blame `a28646ef1` (the PR#1015 SEC-F1 IPv6 fix).

- "The new IPv6 tail check only verifies `tail.starts_with(':')`, but does not validate that the
  remainder is a numeric port. This means malformed authorities like `[::1]:80evil.com` (or `[::1]:`)
  will be treated as host `::1`, which contradicts the comment ('ONLY valid tail is empty or `:port`')
  and can re-open an allow-list bypass because malformed/hostile URLs should fail closed."

### Liaison verification (confirmed on trunk 455dcb7e7)

effect.rs:138 (the PR#1015 fix): `if !(tail.is_empty() || tail.starts_with(':')) { return None; }`. It
accepts ANY tail beginning with `:` — so `[::1]:80evil.com` (tail `:80evil.com`) and `[::1]:` (tail `:`)
PASS and parse as host `::1`. That RE-OPENS the exact allow-list bypass PR#1015 closed for `[::1]evil.com`,
just moved past the colon — a `HostIn(["::1"])` grant authorizes `http://[::1]:80evil.com/`. My PR#1015
fix was incomplete: it checked "tail starts with `:`" but not "tail IS `:` + digits". Fix (fail-closed):
the tail after `]` must be empty OR match `:` followed by 1+ ASCII digits and nothing else (parse the port,
reject `:80evil.com`, `:`, `:0x`). SEC-F1 over-authorize — the never-regress direction.

## Comments 2 (verbatim) — blob.rs:154 (PR#1018 id 3696172722) + blob.rs:147 (PR#1019 id 3696188195) ⚠ DATA-LOSS
blame `a28646ef1` (the PR#1015 blob Windows-rename fix). Same finding, two PRs.

- (PR#1018) "In the Windows rename fallback, any rename error where `path.exists()` is true triggers
  deletion of the existing target and a retry. That's unsafe: `rename` can fail for reasons other than
  'target exists' (permission, IO error, path issues), and in those cases this code can delete a valid
  blob and still fail the rename, losing data. Only attempt the remove+retry when the error kind
  indicates the target already exists (and keep temp cleanup on failure)."
- (PR#1019) same, phrased for `put`.

### Liaison verification (confirmed on trunk 455dcb7e7)

The PR#1015 Windows-rename fix's fallback (blob.rs:~148-154) removes `&path` + retries whenever the rename
errored AND `path.exists()`. But `rename` can fail for PERMISSION / IO / cross-filesystem reasons with the
target still present — in those cases it deletes a VALID existing blob then still fails the retry → DATA
LOSS of a previously-good blob. My PR#1015 fix over-broadened the remove condition. Fix: only remove+retry
when the error KIND specifically indicates "destination exists" (`AlreadyExists`, or a Windows-specific
os-error code — Rust doesn't expose a portable "dest exists" for rename, so gate narrowly + keep the temp
cleanup on all failure paths; on a non-exists error, just clean tmp + return Err WITHOUT deleting path).

## Comment 3 (verbatim) — log_store.rs:121 (PR#1019, id 3696188204) — flush ≠ durable
blame `4be360b0f` "LogStore::truncate_to — heal a torn/corrupt tail".

- "`truncate_to` claims the truncation is made 'durable' by calling `flush()`, but `File::flush()`
  doesn't provide durability guarantees (and is effectively a no-op for unbuffered `File`). If the intent
  is crash-safety for the heal step, use `sync_data()`/`sync_all()`. Otherwise, adjust the doc comment to
  avoid promising durability."

### Liaison verification (confirmed on trunk 455dcb7e7)

`truncate_to` (log_store.rs:118) calls `self.file.flush()?` and the doc claims durability. `std::fs::File`
is unbuffered so `flush()` is a no-op (it flushes a Rust-level buffer that doesn't exist for File) — it
does NOT fsync to disk. For a crash-safety HEAL step (truncating a torn tail so future appends survive
recovery), a crash right after could lose the truncation. Fix: `sync_data()` (or `sync_all()`) for real
durability, OR soften the doc to not promise it. Given it's the recovery-heal path, `sync_data()` is the
right call.

Owner: **v-agent-harness** (`cdz-kernel`). effect.rs + blob.rs are REGRESSIONS in the liaison's own PR#1015
fix (a28646ef1) — the SEC tail-check + the blob-remove condition were both too loose; both are
security/data-integrity, prioritize. log_store flush→sync_data for the heal durability.
