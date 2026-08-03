# PR #1291 review comments — cdz/tests/doc_at_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1291 (PR: "cand: v-cdz-tooling — 80559eb43").
VERIFIED against the diff — both are real (diff lines 36-37 have the `.unwrap()`s).

## 1. `path.to_str().unwrap()` panics on non-UTF-8 temp path (amazon-q, doc_at_cli.rs:31) — test-robustness
> The `.unwrap()` on `path.to_str()` will panic if the temporary directory path contains invalid
> UTF-8. This can occur on systems with non-UTF-8 filesystem paths.

Same class as #1248 (which used `to_string_lossy()`): use `to_string_lossy().into_owned()` (or
`.expect("temp path must be valid UTF-8")` at minimum) so a non-UTF-8 temp path doesn't turn the test
into a panic.

## 2. `std::fs::write(&path, src).unwrap()` → opaque panic on write failure (amazon-q, doc_at_cli.rs:30) — test-robustness
> Using `.unwrap()` for file write operations will panic if the write fails (e.g., disk full,
> permission issues). Replace with `.expect()` to provide a clearer test failure message.

`.expect("write test file")` gives a diagnosable failure instead of a bare unwrap panic. (Matches the
`create_dir_all(&dir).expect("mkdir")` style already one line up in the same helper.)
