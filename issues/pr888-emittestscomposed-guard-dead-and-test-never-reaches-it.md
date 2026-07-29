# PR#888 review comments — EmitTestsComposed stem-collision guard is effectively dead + its test never reaches it (⚠ v-rust-backend)

Mirrored from GitHub PR#888 review comments (Copilot), ids `3671094628` (compile.rs:512) + `3671094671`
(tests.rs:73644). Both `rcdzc`, both from `daf3ad83f` "rcdzc: Option C (c)(iii)c — EmitTestsComposed
driver". Interlinked: the guard doesn't enforce its stated constraint, AND the regression test doesn't
exercise it. This is the rcdzc-side cousin of the PR#881 stem-collision correctness bug (which
v-cdz-tooling fixed on the `cdz` CLI side via a single-dir gate) — worth care, not just a nit.

## Comments (verbatim)

- (id 3671094628, compile.rs:512) "The `EmitTestsComposed` 'single-dir/no-stem-collision' guard doesn't
  actually enforce the stated constraint. It only checks for duplicate `db.file_path` strings (and
  missing paths), but linked packages already reject duplicate `ast` file names at link time, and this
  code never checks that all test files share one directory. As written, the guard is effectively dead
  for multi-file packages and the diagnostic text about 'single-directory' / 'stems across directories'
  is misleading."
- (id 3671094671, tests.rs:73644) "This test constructs two `ast` artifacts with the same name (both
  `\"t\"`). The package linker rejects duplicate file names up-front as an ambiguous import target, so
  this case never reaches the `EmitTestsComposed` guard logic in `compile.rs`. As a result, the test can
  pass (no provider emitted) for the wrong reason and won't protect the intended single-dir/stem-collision
  behavior."

## Liaison verification (both confirmed on trunk fc2b91731)

1. compile.rs:501-511 — the guard computes `stem_collision` by inserting each `named_files` entry's
   `db.file_path(i)` STRING (via `db.file_path(i).map(|p| p.to_string())`) into a HashSet and checking
   for a dup, plus `all_filed` (every bucket has a link path). The DIAGNOSTIC (512-516) promises "every
   `@test` in a distinctly-named (single-directory) file … two files sharing an import STEM ACROSS
   DIRECTORIES would collide". But: (a) it dedups the FULL path string, which is inherently unique per
   file — so `stem_collision` over full paths ~never fires from a real multi-dir bucket; (b) it never
   checks directory-sameness at all; (c) it never reduces to a STEM. So the guard as written does NOT
   enforce "single directory" or "no stem collision" — the exact PR#881 hazard (two `a/t.cdz`, `b/t.cdz`
   sharing stem `t`) is what it's supposed to catch but doesn't. The diagnostic text is misleading about
   what's enforced.
2. tests.rs:73638-73644 — the test passes TWO `Artifact::KIND_AST` both NAMED `"t"` (comment: "Two
   DISTINCT files that share the import stem `t` (as a multi-dir `a/t` + `b/t` would)"), then asserts
   zero `component-provider` emitted. But if the package LINKER rejects two same-named `ast` artifacts as
   an ambiguous import target BEFORE `EmitTestsComposed`'s guard runs, the "no provider" assertion holds
   for the WRONG reason (link rejected, guard never consulted) — so the test does not actually protect
   the stem-collision behavior. (Owner should confirm whether the linker does reject here; if it does,
   the test is vacuous.)

Net: on the rcdzc composed path, the single-dir/stem-collision protection may be ILLUSORY — the guard
doesn't check what it claims, and the test that should catch that doesn't reach it. Suggested (owner's
design call): make the guard actually key on (directory, stem) — decline when two buckets share a file
STEM regardless of directory (or when they span >1 directory, matching the `cdz` CLI single-dir gate
v-cdz-tooling landed for PR#881, `5e78bcbe7`) — and rewrite the test to feed two distinctly-NAMED ast
artifacts whose STEMS collide (e.g. names `"a/t"` and `"b/t"`) so it reaches the guard rather than
tripping the linker's dup-name check.

Owner: **v-rust-backend** (`rcdzc` Option-C `EmitTestsComposed`, `daf3ad83f`). Correctness/test-coverage
— the guard's real behavior + the test's reachability need verifying, not just a comment reword. Cross-ref
the PR#881 `cdz`-side fix (v-cdz-tooling `5e78bcbe7`, single-parent-dir gate) for the intended semantics.
