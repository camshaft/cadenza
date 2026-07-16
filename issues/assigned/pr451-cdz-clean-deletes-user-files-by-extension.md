# PR review comments — mirrored from GitHub PR #451 (Copilot inline) — DATA LOSS

- **PR:** #451 "fleet: seventy-first batch (…, cdz clean, …)" (MERGED)
- **File:** `implementation/seed/crates/cdz/src/main.rs` (artifact selection @969, read_dir error @974)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592990897, 3592990911
- **Links:** https://github.com/camshaft/cadenza/pull/451#discussion_r3592990897 , #discussion_r3592990911

## Comments (verbatim)
> `cdz clean` currently treats any file in the project directory ending with `.wasm`/`.rs`/`.dwarf` as a build artifact. That can delete user-authored files (e.g., Rust helpers, checked-in `.wasm` assets) and contradicts the surrounding docs that claim unrelated outputs are never touched. Consider restricting deletion to only the exact filenames `rcdzc` emits for this project (output name) plus known temp patterns like `.cdz-run-*.wasm` and `link-map.txt`.
> If `read_dir` fails (permissions, missing dir, transient IO), the current code silently proceeds with an empty target list and prints "nothing to clean", masking the underlying error.

## Liaison triage — CONFIRMED against trunk — DATA-LOSS RISK
Confirmed in cdz/src/main.rs: `cdz clean` marks `is_artifact = name.ends_with(".wasm") ||
name.ends_with(".rs") || name.ends_with(".dwarf") || name == "link-map.txt"` for EVERY file in the
project dir, then deletes each. So a user-authored `.rs` (Rust helper) or a checked-in `.wasm` asset in
the project directory is SILENTLY DELETED by `cdz clean` — destructive, and it contradicts the docs
("unrelated outputs are never touched"). `.rs` is especially dangerous. Plus a `read_dir` error is
swallowed → "nothing to clean" masks a real failure. FIX: restrict deletion to the exact filenames
`rcdzc` emits for this project (the output name) + known temp patterns (`.cdz-run-*.wasm`,
`link-map.txt`), NOT a blanket extension sweep; and surface a `read_dir` error instead of printing
"nothing to clean". cdz-tooling (v-cdz-tooling). Fix on `trunk` — worth prioritizing (data loss).
Quotes + links in queue file.
