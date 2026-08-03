# PR #1369 review residual — 12-metaprogramming.sexp eval-symbol docstring 'decline'→'rejection'

github-liaison note 21528: Copilot flagged 12-metaprogramming.sexp:885 — after the #1323/#1369 title
rename ('declines CDZ0101'→'is rejected'), the docstring still said "The decline is at the QUOTE" +
referenced "the no-runtime-AST-interpreter decline above". Since the case outcome is (error CDZ0101)
(coded), "decline" blurs it into the genuine bare-(declines) family.

## Done
- Swept both "decline"→"rejection" (neighbor is also coded CDZ0101). Sibling (quote #"hi") KEEPS
  "declines" (its outcome IS bare (declines)).
- Committed 988ce60a4 (fix-forward on 2b7256489). Docstring-only; gate PASS wasm/rust/rust-async,
  corpus_roundtrip 3/3, no baseline change.

## HOLD / disposition
- STACK-DEPENDENT: edits the same case 2b7256489 (PR #1369, at CI) introduces → must land AFTER #1369.
- COSMETIC docstring → per no-standalone-doc-polish directive + baseline-lane congestion, do NOT send
  a standalone MR. FOLD into the batched corpus MR (with adv-51 + pr1216 [+ adv-50-residual + adv-53
  once CDZ0216/#f49692f13 lands]) once #1369 lands and sync unblocks.
- On sync: 988ce60a4 replays clean onto the new base (its parent 2b7256489 will have landed).
