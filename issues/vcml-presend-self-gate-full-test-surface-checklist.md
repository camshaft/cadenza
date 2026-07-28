# CHECKLIST (v-compiler-ml, self): the PRE-SEND self-gate — run the FULL @test surface, not just the touched files

Motivated by TWO consecutive self-host rejects of the SAME forward-ref MR (2026-07-22): reject 1 caught 5
eval-db + 4 other db-file stale pins; reject 2 caught 2 MORE stale pins in `sread.cdz` — a file I had not
re-run. The gate corpus (`cargo xtask gate --check`) covers NONE of the compiler-ml `.cdz` @test suites, so a
green `--check` is NOT evidence the self-host tests pass. pr-sync runs `cdz test implementation/compiler-ml`
(the FULL 33-file @test surface) and rejects on any failure. So my self-gate MUST match that.

## THE RULE
Before sending ANY compiler-ml MR that changes a Node/Core shape, the NApp/def-table contract, the reader
(`sread.cdz`), or any resolve/infer/lower/eval arm: run `cdz test` on the FULL compiler-ml @test surface and
confirm 0 failures. A contract change invalidates AST-shape and decline PINS scattered across many files, not
just the file you edited.

## Binary
Use the WORKTREE-LOCAL binary: `.claude/worktrees/v-compiler-ml/target/release/cdz` (it has the `test`
subcommand and reads compiler-ml `.cdz` sources LIVE at runtime — source edits reflect without a rebuild).
- The SHARED workspace binary (`<repo>/target/release/cdz`) is often STALE (no `test` subcommand).
- `cargo build -p cdz` writes to the SHARED target, which may NOT update the worktree-local binary — so just
  reuse the existing worktree-local one; a source-only edit needs no rebuild (run-ml/test read sources live).
- Rust-layer changes (rare for me — compiler-ml is `.cdz`) DO need a rebuild; then find where the binary landed.

## The surface, split by cost (run all; 0 failures required)
FAST pure-unit (~1-2s each, no source-pipeline compile) — run inline:
  db, db-state, db-demand, int-width, int-type, ty, ty-eq, ty-bridge, unify, unify-ty, apply-ty,
  infer, infer-let, tycheck, type-env, type-scheme
MODERATE hand-arena unit (a few wasm compiles each):
  parse-db, resolve-db, infer-db, lower-db, eval-db
SLOW run-src (a FULL source→wasm pipeline compile PER @test; minutes each — run in BACKGROUND):
  db-eval, db-infer, db-lower, db-resolve, conformance-db, conformance-db-cx, sread, sread-eval,
  sread-eval-ann, sread-eval-fns, emit-db
  (sread-eval is the fattest ~8min, nearest the 12min per-file gate cap — don't grow it; use a sibling.)

## Highest-risk files for a READ-TIME/contract change (run these FIRST)
sread (reader shape pins) · sread-eval / sread-eval-fns / sread-eval-ann (source-pipeline e2e) ·
the 4 hand-arena db files (eval/lower/infer/resolve-db build NApp directly with literal node-ids) ·
conformance-db (differential-vs-rcdzc decline/run pins). These are where a call-vs-NBin or forward/unknown
decline PIN lives; grep alone is NOT proof (comments/prose match too) — RUN them.

## Why grep-audit is insufficient (learned the hard way)
`grep "is-bin-op.*0 - 1"` finds prose comments and already-migrated tests too. A test that asserts an AST SHAPE
(`is-bin-op(tree, root, -1)` for a call, or a `None`-branch `trap()`) only reveals its staleness by RUNNING —
the trap compiles to `wasm unreachable`, which is the tell: "wasm unreachable" in a compiler-ml @test = an
ASSERTION failed (a decline/shape pin no longer holds), NOT a miscompile.
