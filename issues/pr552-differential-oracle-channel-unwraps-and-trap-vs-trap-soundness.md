# pr552 — cdz-smith differential oracle: channel .unwrap() hardening + Trap-vs-Trap soundness (5 comments)

Mirrored from GitHub PR #552 review comments.
PR: https://github.com/camshaft/cadenza/pull/552 (8-MR publish batch, MERGED to trunk)
File: `implementation/seed/crates/cdz-smith/src/differential.rs` — the differential oracle.

## Soundness — id 3607207128 [Copilot] differential.rs:120 — Trap-vs-Trap hides rust-artifact-error
> `parse_rust_verdict` maps `error ...` (rustc/emit artifact failure) to
> `Side::Trap("rust-artifact-error: …")`, but `compare` treats Trap-vs-Trap as unconditional
> agreement. That means a Rust backend miscompile that prevents compilation can be silently ignored
> whenever the wasm side also traps (even though the artifact error is a real compiler bug).
> Consider treating any `rust-artifact-error:` trap as a mismatch even in Trap-vs-Trap comparisons
> so these are always surfaced.

REAL oracle-soundness gap: the whole point of the differential oracle is to catch rust-vs-wasm
divergence; a rust-artifact-error masquerading as a Trap that agrees with a wasm trap defeats it.
Worth the fuzzer owner's judgment.

## Robustness — 4x amazon-q: .unwrap() on async channel ops panics the oracle daemon
amazon-q frames these as "CWE-248 DoS security vulnerabilities" — OVERBLOWN for an internal
fuzzer/oracle daemon (not a network-facing service), but the underlying point is legit: an
`.unwrap()` on channel recv/send panics the daemon on channel close. VERIFY loci (amazon-q source,
line numbers shifted post-merge). Reasonable to harden to `?`/`map_err`.
- id 3607196962 differential.rs:254 — `result_rx.recv().await.unwrap()` (receive differential result)
- id 3607196969 differential.rs:277 — `update_rx.recv().await.unwrap()` (receive dependency updates)
- id 3607196972 differential.rs:320 — `result_tx.send(result).await.unwrap()` (send differential result)
- id 3607196976 differential.rs:335 — `update_tx.send(updates).await.unwrap()` (send dependency updates)

## Owner
All in `cdz-smith/src/differential.rs` = fuzzer/cdz-smith territory (PM already routed PR#551's
differential-oracle findings to the fuzzer). Soundness (#120) is the substantive one; the 4 unwraps
are a robustness sweep.
