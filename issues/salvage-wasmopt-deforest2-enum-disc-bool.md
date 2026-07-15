# Salvaged commit — evaluate for landing (v-wasm-opt territory)

Tag: `salvage/wasmopt-deforest2`  (commit 7363c81d), branch `wasmopt-deforest2`
Subject: rcdzc wasm: an if between two disc-{0,1} enum variants materializes the bool (no select)

Unmerged wasm-output optimization from the old wasmopt-deforest2 worktree. NOT in trunk. This is
v-wasm-opt's territory: evaluate against the current emitted-wasm optimizer, cherry-pick if still a
win (`git cherry-pick salvage/wasmopt-deforest2`), gate (behavior corpus must stay green), send
pr-sync a merge-request. If already covered, mark .REJECTED.md + drop the tag/branch.
