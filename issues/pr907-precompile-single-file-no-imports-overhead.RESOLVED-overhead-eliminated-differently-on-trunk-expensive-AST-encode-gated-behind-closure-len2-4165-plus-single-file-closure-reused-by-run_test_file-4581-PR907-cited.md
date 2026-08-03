# PR#907 review comment — single-file no-imports cdz test does composed-setup before falling back (v-cdz-tooling, perf)

Mirrored from GitHub PR#907 review comment (Copilot), id `3679422104`.
File: `implementation/seed/crates/cdz/src/main.rs:3933` — v-cdz-tooling. Blame `cce22e4b5` "cdz test:
single-file run reuses a warmed provider cache (single-file-local-verify win)".

## Comment (verbatim)

- (id 3679422104, cdz/src/main.rs:3933) "`precompile_tests_per_file` no longer returns early for
  `files.len() == 1`, which means a common `cdz test <file>` (with no imports) now does composed-path
  setup work before immediately falling back (and then `run_test_file` re-parses/compiles as usual).
  This is behavior-neutral but adds overhead on the single-file/no-imports path; a cheap early heuristic
  gate can restore the old fast-path while still allowing single-file *with imports* to reach the
  provider-cache logic."

## Liaison verification (confirmed on trunk b81cd9cba)

`precompile_tests_per_file` (main.rs:3926): the old `if files.len() < 2 { return }` early-out was
DELIBERATELY removed (`cce22e4b5`) so a single file CAN reach the cross-invocation provider cache (the
single-file-local-verify win — a witness against the ~1360-def self-host closure). The real "nothing to
do" test now runs AFTER gathering the import closure: `asts.len() < 2` (a self-contained file has no
cross-file closure member → falls back to the byte-identical per-file compile). Copilot's point is
correct: a SELF-CONTAINED single file (no imports — the common `cdz test <file>` case) now pays the
closure-GATHER setup (load_import_closure per file, AST-encode) before that `asts.len() < 2` check fails
and it falls back — pure overhead on the hot no-imports path (which the old `< 2` gate skipped). Fix
(Copilot's, sound): a CHEAP early heuristic BEFORE the gather — e.g. "single file AND it has no
`(import …)` clauses" → return `Precompiled::default()` immediately (restoring the fast-path), while a
single file WITH imports still proceeds to the provider-cache logic. Behavior-neutral (both paths already
fall back / cache identically); this only trims wasted setup on the no-imports single-file run. Owner
knows the cheapest import-presence probe (parse already done? a token scan?).

Owner: **v-cdz-tooling** (`cdz` CLI provider cache, `cce22e4b5`). Add an early no-imports single-file
fast-path gate before the closure gather.
