//! `cargo xtask lint-mandates` — gate-level enforcement of the operator's mechanizable STANDING
//! MANDATES over the Rust compiler codebase, so a violating MR fails the gate rather than being caught
//! post-hoc by the operator reading diffs. Mirrors `emoji_free_lint` (a cheap source-scan, ONE native
//! `check()` step, no per-crate rebuild); v-fleet-tooling wires it into `cargo xtask check` + the nix
//! `localGate`, exactly as the NO-emoji mandate is enforced today.
//!
//! This is a RUST-SOURCE lint (scans `implementation/**/*.rs`) — distinct from `cadenza-syntax`'s
//! `cdz lint`, which matches CADENZA programs. The mandates it enforces are about the Rust codebase, so
//! Rust source is the right surface.
//!
//! DENY rules (a hit fails the gate), each with a scoped `mandate:allow` escape hatch for a justified
//! exception (a hex-parse at a wire boundary, a component E2E that cannot be an in-crate test):
//!  - **no-integration-tests** — a NEW `tests/*.rs` cargo integration test (operator prefer-unit-tests:
//!    a `tests/*.rs` is a separate binary + linkage + slow compile + separate nix derivation). The 97
//!    pre-existing files are grandfathered via `mandate-integration-test-allowlist.txt`; a NEW one not on
//!    the allowlist is denied. Going-forward, not a fleet-reddening full-tree ban.
//!
//! The syn-based DENY rules (no-hex-except-tracing, no-thin-wrapper-fns, no-hard-coded-kernel-names) and
//! the WARN-level owned-String-where-Arc heuristic are built as follow-on increments — this lands the
//! first (exact, unambiguous) rule + the framework.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A mandate violation: the file it is in and a human-readable reason. Rendered `path: reason`.
pub struct Violation {
    pub file: PathBuf,
    pub reason: String,
}

/// Run every mechanizable mandate check over `implementation/**/*.rs` under `repo`. Returns the
/// violations (empty = clean). The caller (the `lint-mandates` subcommand / the `check()` step) fails
/// the gate when any are returned.
pub fn lint_mandates(repo: &Path) -> Result<Vec<Violation>, String> {
    let mut out = Vec::new();
    out.extend(no_new_integration_tests(repo)?);
    // Follow-on: no_hex_except_tracing, no_thin_wrapper_fns, no_hard_coded_kernel_names (syn-based).
    Ok(out)
}

/// The `no-integration-tests` mandate: a `tests/*.rs` cargo integration test not on the grandfather
/// allowlist is a violation. `tests/*.rs` = a separate test binary (extra linkage, slow compile,
/// separate nix derivation) — the operator prefers in-crate `#[cfg(test)]` unit tests. Pre-existing
/// files are allowlisted (`mandate-integration-test-allowlist.txt`); a genuinely-integration test (a
/// built-component / binary E2E that cannot be in-crate) is added to the allowlist with a justification.
fn no_new_integration_tests(repo: &Path) -> Result<Vec<Violation>, String> {
    let allow = load_allowlist(repo)?;
    let impl_root = repo.join("implementation");
    let mut rs = Vec::new();
    collect_rs_files(&impl_root, &mut rs).map_err(|e| {
        format!(
            "cannot enumerate {} for the mandate lint: {e}",
            impl_root.display()
        )
    })?;
    let mut out = Vec::new();
    for f in rs {
        if !is_integration_test_path(&f) {
            continue;
        }
        // Compare by the repo-relative path (the allowlist's form).
        let rel = f.strip_prefix(repo).unwrap_or(&f);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // EXCLUDE VENDORED / third-party trees (v-ft 2026-08-09, caught by the pre-wire standalone
        // validation): a `reference/` path segment marks vendored code we do NOT author (e.g.
        // implementation/music/reference/euphony-rs/…/tests/api.rs) — the operator prefer-unit-tests
        // mandate is OUR discipline, not applicable to a third-party crate's own test layout, and
        // enumerating every vendored tests/*.rs into the allowlist is churn (new vendored files would
        // re-red). Skipping the vendored tree is the correct scope + is future-proof. Without this the
        // lint reds on a PRE-EXISTING vendored file every MR → blocks the whole fleet (localGate is the
        // sole gate) — the exact fleet-block failure mode the pre-wire validation exists to catch.
        if is_vendored_path(&rel_str) {
            continue;
        }
        if !allow.contains(&rel_str) {
            out.push(Violation {
                file: f.clone(),
                reason:
                    "new cargo integration test (tests/*.rs) — the operator mandate prefers in-crate \
                     #[cfg(test)] unit tests (a tests/*.rs is a separate binary + linkage + slow compile \
                     + separate nix derivation). Move the coverage into a #[cfg(test)] mod in the crate; \
                     if this is a genuine component/binary E2E that cannot be in-crate, add its path to \
                     xtask/mandate-integration-test-allowlist.txt with a one-line justification"
                        .to_string(),
            });
        }
    }
    Ok(out)
}

/// Whether a repo-relative path is under a VENDORED / third-party tree the mandate must NOT police.
/// A `reference/` path segment marks vendored code we don't author (e.g.
/// `implementation/music/reference/euphony-rs/…`). Our prefer-unit-tests (and other) mandates are our
/// own discipline; a third-party crate's test layout is not ours to change, and allowlisting each
/// vendored file is churn (new vendored files would re-red). Pure over the slash-normalized rel path so
/// the exclusion is unit-tested.
fn is_vendored_path(rel_slash: &str) -> bool {
    rel_slash.split('/').any(|seg| seg == "reference")
}

/// Is `path` a cargo INTEGRATION-test source — a `.rs` file under a crate's top-level `tests/` directory?
/// A `tests/` dir directly under a crate root (`…/<crate>/tests/**/*.rs`) is the cargo integration-test
/// surface. (A `tests` MODULE inside `src/` is `src/…/tests.rs` or an inline `mod tests`, NOT this —
/// those are the in-crate unit tests we WANT, so only a `/tests/` path segment counts.)
fn is_integration_test_path(path: &Path) -> bool {
    if path.extension().is_none_or(|x| x != "rs") {
        return false;
    }
    // A `tests` component that is NOT inside `src/` — i.e. a crate-root `tests/` dir. Cargo only treats
    // `<crate>/tests/*.rs` as integration tests; a `src/**/tests/` would be an ordinary module path.
    let comps: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let Some(tests_ix) = comps.iter().position(|&c| c == "tests") else {
        return false;
    };
    // Must not be under a `src/` before the `tests` segment (that would be a module named tests).
    !comps[..tests_ix].contains(&"src")
}

/// Load the grandfather allowlist (`xtask/mandate-integration-test-allowlist.txt`): one repo-relative
/// path per line, `#` comments + blank lines ignored. Missing file = an empty allowlist (every
/// integration test would then be flagged — the file is expected to exist).
fn load_allowlist(repo: &Path) -> Result<BTreeSet<String>, String> {
    let p = repo.join("xtask/mandate-integration-test-allowlist.txt");
    let text = std::fs::read_to_string(&p)
        .map_err(|e| format!("cannot read the mandate allowlist {}: {e}", p.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.replace('\\', "/"))
        .collect())
}

/// Recursively collect `.rs` files under `dir` (skipping `target/`). Mirrors the emoji lint's walker.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_test_path_detection() {
        // A crate-root `tests/` .rs is an integration test.
        assert!(is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz/tests/lint_cli.rs"
        )));
        assert!(is_integration_test_path(Path::new(
            "implementation/seed/crates/cadenza-syntax/tests/suite/corpus_roundtrip.rs"
        )));
        // A `src/**` file — even one named tests.rs or under a `tests` MODULE dir under src — is NOT a
        // cargo integration test (it's an in-crate unit test, which the mandate WANTS).
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cadenza-syntax/src/query.rs"
        )));
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/foo/src/tests.rs"
        )));
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/foo/src/parser/tests/helpers.rs"
        )));
        // A non-.rs file under tests/ (a fixture) is not flagged.
        assert!(!is_integration_test_path(Path::new(
            "implementation/seed/crates/cdz-kernel/tests/fixtures/reducer.cdz"
        )));
    }

    #[test]
    fn vendored_path_excludes_reference_trees_only() {
        // A `reference/` segment = vendored third-party (not ours to mandate) → excluded.
        assert!(is_vendored_path(
            "implementation/music/reference/euphony-rs/euphony/tests/api.rs"
        ));
        assert!(is_vendored_path(
            "implementation/foo/reference/bar/tests/x.rs"
        ));
        // Our own crates (no `reference/` segment) are NOT excluded — the mandate applies.
        assert!(!is_vendored_path(
            "implementation/seed/crates/cdz/tests/lint_cli.rs"
        ));
        // A substring-but-not-a-segment (e.g. a crate literally named with 'reference' inside) must not
        // match on substring — only a whole path SEGMENT `reference` counts.
        assert!(!is_vendored_path(
            "implementation/seed/crates/reference-impl/tests/x.rs"
        ));
    }

    #[test]
    fn allowlist_parse_ignores_comments_and_blanks() {
        // (Parsing is exercised via `load_allowlist` over the real file in the gate; here pin the
        // filter semantics on a synthetic set the same way.)
        let sample = "# header\n\nfoo/tests/a.rs\n  # indented comment\nbar/tests/b.rs\n";
        let set: BTreeSet<String> = sample
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.replace('\\', "/"))
            .collect();
        assert!(set.contains("foo/tests/a.rs"));
        assert!(set.contains("bar/tests/b.rs"));
        assert_eq!(set.len(), 2, "comments + blanks ignored");
    }
}
