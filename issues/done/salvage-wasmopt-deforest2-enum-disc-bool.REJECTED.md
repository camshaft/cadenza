# Salvaged commit — evaluate for landing (v-wasm-opt territory)

Tag: `salvage/wasmopt-deforest2`  (commit 7363c81d), branch `wasmopt-deforest2`
Subject: rcdzc wasm: an if between two disc-{0,1} enum variants materializes the bool (no select)

Unmerged wasm-output optimization from the old wasmopt-deforest2 worktree. NOT in trunk. This is
v-wasm-opt's territory: evaluate against the current emitted-wasm optimizer, cherry-pick if still a
win (`git cherry-pick salvage/wasmopt-deforest2`), gate (behavior corpus must stay green), send
pr-sync a merge-request. If already covered, mark .REJECTED.md + drop the tag/branch.

## v-wasm-opt evaluation (2026-07-15) — REJECTED (already covered)
Evaluated against the current emitted-wasm optimizer. `(if c (A) (B))` on a disc-{0,1} enum
(all-nullary sum = enum-disc) ALREADY emits the optimal branchless form:
`i32.const 0 ; i32.const 1 ; local.get 0 ; select` (4 instrs, no branch) — delivered by the landed
enum-disc if→select fold (cycle 184). Value parity verified (f(true)→A→10). The salvage commit's
"materialize the bool" approach would save at most 1-2 instrs in this narrow case and risks regressing
the current clean, general `select` form. NOT cherry-picked. Recommend dropping tag/branch
`salvage/wasmopt-deforest2` (commit 7363c81d). Renaming this file .REJECTED.md.
