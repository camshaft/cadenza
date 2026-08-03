# PR#881 review comment — cdz test precompile stem-collision → PASS/FAIL misattribution (v-cdz-tooling) ⚠ CORRECTNESS

Mirrored from GitHub PR#881 review comment (Copilot), id `3668419763`.
File: `implementation/seed/crates/cdz/src/main.rs:3924` — `cdz` CLI = v-cdz-tooling's lane. Blame
`5da05cab6` "cdz test: lower the shared closure once per dir via EmitTestsPerFile".

⚠ NOT a nit — a plausible CORRECTNESS bug (wrong PASS/FAIL attribution). Flagged for v-cdz-tooling's
judgment; the fallback does NOT mask it (see below).

## Comment (verbatim)

- (id 3668419763, main.rs:3924) "`precompile_tests_per_file` unions import closures across *all* target
  files and dedupes by `cf.name` (which is `program_name`/file stem). Because `cdz test <dir>` collects
  source files recursively, the input set can span multiple directories that each contain a `lib.cdz` /
  `main.cdz`, etc. In that situation the union compile can silently collide modules with the same stem
  from different directories, and `run_test_file` can then reuse the wrong precompiled component for a
  file (incorrect PASS/FAIL attribution). A safe fix is to only enable the shared-arena precompile when
  all target files are in the same directory (or otherwise group the precompile by directory/namespace)."

## Liaison verification (confirmed on trunk f1ee5c564 — REAL risk)

- `program_name` (main.rs:6080) = `Path::file_stem()` (fallback "main") — a bare STEM, directory-blind.
- `precompile_tests_per_file` (3919): `asts.entry(cf.name.clone()).or_insert_with(...)` dedupes the
  union by `cf.name` = the stem. Two different-directory files both named `lib.cdz` → same stem `lib` →
  the second `.or_insert` is a NO-OP, so ONE `lib`'s AST represents BOTH in the union.
- `run_test_file` (4343): `precompiled.get(&closure[0].name)` — looks its component up by the SAME stem.
  So the emitted-per-file component keyed `lib` could be the OTHER directory's `lib`.
- The BEST-EFFORT fallback only fires when the component is ABSENT / the file wasn't in the union — a
  stem collision yields a PRESENT-but-WRONG component, so the fallback does NOT catch it. Result:
  `cdz test <dir>` over a tree with same-stem files in different subdirs can run the wrong component for
  a file → incorrect PASS/FAIL. (`cdz test <dir>` DOES recurse: the doc says "every source file under it
  (recursively) is run".)

Impact bound: only `cdz test <dir>` over a multi-dir tree with same-stem files; a single-dir package or a
single file is unaffected (single-file already returns empty map → all fallback). Likelihood depends on
whether the current gate corpus / self-host layout has same-stem files across dirs (owner knows).

Suggested fixes (Copilot's, both sound):
1. Gate the shared-arena precompile on ALL target files sharing one directory (else return empty → all
   fall back to the exact per-file compile — safe, just slower for the multi-dir case).
2. Or key the union + lookup by a directory/namespace-qualified name (full relative path stem), not the
   bare file stem, so cross-dir same-stem files stay distinct.

Owner: **v-cdz-tooling** (`cdz/src/main.rs` CLI; `5da05cab6`). Correctness — recommend prioritizing over
the doc-nit queue.
