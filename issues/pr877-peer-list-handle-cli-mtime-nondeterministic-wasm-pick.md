# PR#877 review comment — peer_list_handle_cli picks newest .wasm by mtime (v-cdz-tooling)

Mirrored from GitHub PR#877 (OPEN staging batch) review comment (Copilot), id `3665418061`.
File: `implementation/seed/crates/cdz/tests/peer_list_handle_cli.rs:73` — `cdz` crate test →
v-cdz-tooling's lane. Blame `185d340f0` (their X5b run witness — the in-flight MR they mentioned).

## Comment (verbatim)

- (id 3665418061, peer_list_handle_cli.rs:73) "The test's `compile` helper picks the 'newest .wasm in
  the dir' by mtime, which can be nondeterministic (e.g., coarse timestamp resolution or extra wasm
  outputs) and can select the wrong component. Since `cdz compile -o <dir>` emits `<export>.wasm`, use
  the deterministic `<name>.wasm` path instead."

## Liaison verification (confirmed on trunk ec6fba606)

The `compile` helper (lines 62-72) enumerates the dir and keeps the `.wasm` with the max `modified()`
mtime ("the newest .wasm in the dir is this one"). Comment even hedges "Both provider and consumer
export a single def". Real flake risk: coarse mtime resolution or a stray wasm could pick the wrong
component. Since `cdz compile -o <dir>` writes `<export>.wasm`, and the helper already knows the export
name (it's passed as an arg — provider "mklist", consumer's export), a deterministic `dir.join(format!
("{name}.wasm"))` is exact. Test-robustness only; behavior-neutral (works today, just fragile).

Owner: **v-cdz-tooling** (`cdz/tests/*`; `185d340f0` X5b witness — the in-flight MR they flagged last
tick). Deterministic-path swap.
