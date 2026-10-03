//! `corpus-chapter-gate` — gate a corpus chapter across EVERY execution target that shares its
//! `spec/semantics/.gate-baseline`, reddening if any one reds.
//!
//! One corpus chapter drives four checks that all grade against the SAME baseline: the wasm corpus
//! (`corpus-<stem>`), the cadenza re-emit (`corpus-cadenza-<stem>`), and the rust + rust-async backends
//! (`corpus-rust-<stem>` / `corpus-rust-async-<stem>`). Only the wasm one is a `gate-local` constituent;
//! the other three are advisory/nightly. So a wasm-only pass can mask a re-emit or rust trap on a
//! baseline-`pass` case, and that trap lands latent on main — exactly what SHAPE 342 did (#346). This
//! helper runs all four in ONE `nix build` (which fails if any attr reds), so "gate the chapter" means
//! every baseline-sharing target, not just wasm.

use crate::Paths;
use std::path::Path;
use std::process::Command;
use xtask_support::default_corpus_files;

/// The four baseline-sharing check attrs for one corpus stem, in gate order: the wasm corpus, the cadenza
/// re-emit, then the rust + rust-async backends. All grade against the same `spec/semantics/.gate-baseline`,
/// so gating all four is what stops a wasm-only pass from masking a re-emit/rust trap (#346). Pure.
pub(crate) fn chapter_target_attrs(stem: &str) -> Vec<String> {
    vec![
        format!("corpus-{stem}"),
        format!("corpus-cadenza-{stem}"),
        format!("corpus-rust-{stem}"),
        format!("corpus-rust-async-{stem}"),
    ]
}

/// Resolve `<chapter>` to the corpus stem(s) it gates, given the `available` corpus stems. An exact stem
/// match resolves to just that stem (the canonical form). A BARE chapter number (all ASCII digits, e.g.
/// `28`) EXPANDS to every stem in that chapter (see [`stem_in_chapter`]) — so `28` gates BOTH
/// `28-compiler-primitives` and `28-wit-abi-boundary`, and `14` gates `14`/`14b`/`14c` — erring toward
/// gating MORE, since an under-gate is the exact bug #346 closes. Any other input is unknown. Returns the
/// matched stems sorted (deterministic output), or an `Err` message listing the available stems. Pure —
/// unit-tested.
pub(crate) fn resolve_stems(chapter: &str, available: &[String]) -> Result<Vec<String>, String> {
    // (1) exact stem match — the canonical form, gates just that one stem.
    if available.iter().any(|s| s == chapter) {
        return Ok(vec![chapter.to_string()]);
    }
    // (2) a bare chapter number expands to every stem in that chapter.
    if !chapter.is_empty() && chapter.bytes().all(|b| b.is_ascii_digit()) {
        let mut matched: Vec<String> = available
            .iter()
            .filter(|s| stem_in_chapter(s, chapter))
            .cloned()
            .collect();
        matched.sort();
        if matched.is_empty() {
            return Err(format!(
                "no corpus chapter matches the number '{chapter}'. Available stems: {}",
                available.join(", ")
            ));
        }
        return Ok(matched);
    }
    // (3) neither a stem nor a bare number.
    Err(format!(
        "'{chapter}' is neither a corpus stem nor a bare chapter number — pass a full stem \
         (e.g. 28-wit-abi-boundary) or a chapter number (e.g. 28). Available stems: {}",
        available.join(", ")
    ))
}

/// Does `stem` belong to the bare chapter `number`? True iff `stem` begins with the digits `number` and
/// the very next character is `-` (e.g. `28-…`) or a lowercase-ASCII variant letter (e.g. `14b-…`). This
/// respects the digit boundary — `2` does NOT match `28-…`, and `14` does NOT match `140-…` — while still
/// catching the `14b`/`14c` letter variants of chapter 14. Pure.
fn stem_in_chapter(stem: &str, number: &str) -> bool {
    let Some(rest) = stem.strip_prefix(number) else {
        return false;
    };
    match rest.bytes().next() {
        Some(b'-') => true,
        Some(b) => b.is_ascii_lowercase(),
        None => false, // a bare number with no `-label` is not a real corpus stem
    }
}

/// The corpus stems available in this repo — the `spec/semantics/*.sexp` file stems, the exact source the
/// nix per-chapter checks are generated from.
fn available_stems(paths: &Paths) -> Vec<String> {
    default_corpus_files(&paths.repo)
        .iter()
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect()
}

/// The current nix system double (e.g. `aarch64-linux`), used to address `.#checks.<system>.<attr>`.
/// Falls back to `aarch64-linux` if nix can't be run.
fn current_system() -> String {
    Command::new("nix")
        .args([
            "eval",
            "--raw",
            "--impure",
            "--expr",
            "builtins.currentSystem",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "aarch64-linux".to_string())
}

/// Why the `rcdzc` lib-test stage ran (or did not) — printed so the decision is never silent, and the
/// discriminant the unit tests pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibTestDecision {
    /// Caller forced it off with `--no-lib-tests`.
    ForcedOff,
    /// Caller forced it on with `--with-lib-tests`.
    ForcedOn,
    /// The diff looked like a decline flip (touched a backend decline predicate or a shared baseline), so
    /// the guard auto-fired.
    AutoFlip,
    /// The diff did not look like a decline flip, so the guard stayed off.
    AutoNoFlip,
    /// The diff could not be inspected (base unresolvable, not a repo, git error); the guard ran anyway
    /// rather than risk MISSING a flip — running the fast native suite is cheaper than a fleet-wide red.
    FailSafe,
}

impl LibTestDecision {
    /// Does this decision run `cargo test -p rcdzc`?
    pub(crate) fn runs_lib_tests(self) -> bool {
        !matches!(
            self,
            LibTestDecision::ForcedOff | LibTestDecision::AutoNoFlip
        )
    }
}

/// Decide whether to run the lib-test guard from the two explicit flags and the changed-file set.
/// `--with-lib-tests` wins over everything (explicit on), then `--no-lib-tests` (explicit off); with
/// neither flag, auto-detect from the diff — `None` for `changed` means the diff was un-inspectable and we
/// fail SAFE (run it). Pure — unit-tested.
pub(crate) fn decide_lib_tests(
    with_lib_tests: bool,
    no_lib_tests: bool,
    changed: Option<&[String]>,
) -> LibTestDecision {
    if with_lib_tests {
        return LibTestDecision::ForcedOn;
    }
    if no_lib_tests {
        return LibTestDecision::ForcedOff;
    }
    match changed {
        None => LibTestDecision::FailSafe,
        Some(c) if is_decline_flip_diff(c) => LibTestDecision::AutoFlip,
        Some(_) => LibTestDecision::AutoNoFlip,
    }
}

/// Does this set of changed (project-root-relative) paths look like a DECLINE FLIP — a change that can turn
/// a declining shape into a crossing one and so orphan a sibling "must-decline" rcdzc `#[test]` (#982)? True
/// iff any path is a backend decline predicate (`implementation/seed/crates/rcdzc/src/backend/<x>/mod.rs`)
/// or a shared execution baseline. A real flip ALWAYS rewrites a `.gate-baseline*` entry (that is what a
/// flip IS — a `todo`/`decline` graded `pass`), so the baseline test alone catches every flip; the
/// backend-predicate test is a belt-and-suspenders for a predicate edit staged apart from its baseline.
/// Erring toward MORE is deliberate: an under-trigger reopens the exact fleet-wide-red gap, while an
/// over-trigger only runs the fast native rcdzc suite on a non-flip edit that happens to touch these files.
/// Pure — unit-tested.
pub(crate) fn is_decline_flip_diff(changed: &[String]) -> bool {
    changed
        .iter()
        .any(|p| is_shared_execution_baseline(p) || is_backend_decline_predicate(p))
}

/// The three execution baselines a flip rewrites: the shared (wasm/cadenza) baseline and the two rust
/// backend baselines. (`.quote-gate-baseline` is a syntactic round-trip gate, not an execution outcome, so a
/// change there is not a decline flip.) Pure.
fn is_shared_execution_baseline(path: &str) -> bool {
    matches!(
        path,
        "spec/semantics/.gate-baseline"
            | "spec/semantics/.gate-baseline-rust"
            | "spec/semantics/.gate-baseline-rust-async"
    )
}

/// Is `path` a backend decline predicate — `implementation/seed/crates/rcdzc/src/backend/<backend>/mod.rs`,
/// exactly one backend segment then `mod.rs` (e.g. `.../backend/rust/mod.rs`, `.../backend/wasm/mod.rs`)?
/// Pure.
fn is_backend_decline_predicate(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("implementation/seed/crates/rcdzc/src/backend/") else {
        return false;
    };
    matches!(rest.split_once('/'), Some((seg, "mod.rs")) if !seg.is_empty() && !seg.contains('/'))
}

/// The project-root-relative paths changed on this worktree relative to the integration base, used to decide
/// whether the lib-test guard auto-fires. Union of the committed changes since the merge-base with the first
/// resolvable integration base (`origin/main`, then `main`, then `trunk`) and the current working-tree diff,
/// so an uncommitted flip is caught too. Returns `None` if no base resolves or git errs — the caller then
/// fails SAFE and runs the guard rather than risk missing a flip.
fn changed_files_vs_base(repo: &Path) -> Option<Vec<String>> {
    let base = ["origin/main", "main", "trunk"]
        .into_iter()
        .find(|r| git_ref_exists(repo, r))?;
    let mut files: Vec<String> = Vec::new();
    // Committed on this branch since the merge-base (three-dot), plus the uncommitted working-tree diff.
    for args in [
        vec!["diff", "--name-only", &format!("{base}...HEAD")],
        vec!["diff", "--name-only", "HEAD"],
    ] {
        let out = Command::new("git")
            .args(&args)
            .current_dir(repo)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        files.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string),
        );
    }
    files.sort();
    files.dedup();
    Some(files)
}

/// Does the git ref `r` resolve in `repo`? Used to pick the integration base.
fn git_ref_exists(repo: &Path, r: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", r])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run the chapter gate: resolve `<chapter>` to its stem(s), PRINT the resolved list up front (so an
/// expansion is never silent), then one `nix build --no-link` over every stem's four baseline-sharing
/// check attrs. A multi-attr `nix build` fails if ANY attr reds — the natural fan-out. After a GREEN corpus
/// build it runs the `rcdzc` lib unit tests (`cargo test -p rcdzc`) when [`decide_lib_tests`] says to — the
/// guard a decline-flip land needs, since the four corpus attrs never build the rcdzc lib tests where a
/// stale "must-decline" `#[test]` lives (#982). The guard AUTO-fires on a decline-flip-shaped diff (a
/// backend decline-predicate or `.gate-baseline*` touch) so a flip author cannot forget to opt in; the flags
/// only override that. This process exits with the first RED stage's code. Never returns.
pub(crate) fn run(paths: &Paths, chapter: &str, with_lib_tests: bool, no_lib_tests: bool) -> ! {
    let available = available_stems(paths);
    let stems = match resolve_stems(chapter, &available) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("xtask corpus-chapter-gate: {e}");
            std::process::exit(2);
        }
    };
    // Decide the lib-test stage BEFORE the (slow) corpus build so the plan is printed up front. Auto-detect
    // reads the diff only when neither flag forces the outcome.
    let decision = if with_lib_tests || no_lib_tests {
        decide_lib_tests(with_lib_tests, no_lib_tests, None)
    } else {
        decide_lib_tests(false, false, changed_files_vs_base(&paths.repo).as_deref())
    };
    let system = current_system();
    eprintln!(
        "corpus-chapter-gate {chapter} -> gating {} chapter(s): {} \
         [x4 targets each: corpus-/corpus-cadenza-/corpus-rust-/corpus-rust-async-<stem>]{}",
        stems.len(),
        stems.join(", "),
        if decision.runs_lib_tests() {
            format!(" + cargo test -p rcdzc [{decision:?}]")
        } else {
            format!(" [lib tests skipped: {decision:?}]")
        }
    );
    let attrs: Vec<String> = stems.iter().flat_map(|s| chapter_target_attrs(s)).collect();
    let mut build_args: Vec<String> = vec!["build".to_string(), "--no-link".to_string()];
    build_args.extend(attrs.iter().map(|a| format!(".#checks.{system}.{a}")));
    eprintln!("+ nix {}", build_args.join(" "));
    let status = Command::new("nix")
        .args(&build_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("xtask corpus-chapter-gate: could not invoke `nix build`: {e}");
            std::process::exit(1);
        });
    // A red corpus build is the whole result — exit now, before the (slower) lib-test stage.
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    if decision.runs_lib_tests() {
        eprintln!("+ cargo test -p rcdzc");
        let test_status = Command::new("cargo")
            .args(["test", "-p", "rcdzc"])
            .current_dir(&paths.repo)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("xtask corpus-chapter-gate: could not invoke `cargo test -p rcdzc`: {e}");
                std::process::exit(1);
            });
        std::process::exit(test_status.code().unwrap_or(1));
    }
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<String> {
        [
            "05-compound-types",
            "14-effects-and-handlers",
            "14b-effects-and-handlers",
            "14c-effects-and-handlers",
            "25-verification",
            "25-verification-neighbors",
            "28-compiler-primitives",
            "28-wit-abi-boundary",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn chapter_target_attrs_fans_the_four_baseline_sharing_targets() {
        // The wasm/cadenza/rust/rust-async targets that share one .gate-baseline — the set #346 requires.
        assert_eq!(
            chapter_target_attrs("28-wit-abi-boundary"),
            vec![
                "corpus-28-wit-abi-boundary",
                "corpus-cadenza-28-wit-abi-boundary",
                "corpus-rust-28-wit-abi-boundary",
                "corpus-rust-async-28-wit-abi-boundary",
            ]
        );
    }

    #[test]
    fn resolve_stems_exact_stem_is_canonical() {
        assert_eq!(
            resolve_stems("28-wit-abi-boundary", &sample()).unwrap(),
            vec!["28-wit-abi-boundary".to_string()]
        );
    }

    #[test]
    fn resolve_stems_bare_number_expands_to_every_stem_in_the_chapter() {
        // 28 gates BOTH 28-* stems — the exact ambiguity #346 is about (a wasm-only gate of one masks the other).
        assert_eq!(
            resolve_stems("28", &sample()).unwrap(),
            vec![
                "28-compiler-primitives".to_string(),
                "28-wit-abi-boundary".to_string(),
            ]
        );
        // 14 pulls in the b/c letter-variant chapters.
        assert_eq!(
            resolve_stems("14", &sample()).unwrap(),
            vec![
                "14-effects-and-handlers".to_string(),
                "14b-effects-and-handlers".to_string(),
                "14c-effects-and-handlers".to_string(),
            ]
        );
        // 25 -> both verification stems.
        assert_eq!(
            resolve_stems("25", &sample()).unwrap(),
            vec![
                "25-verification".to_string(),
                "25-verification-neighbors".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_stems_respects_the_digit_boundary() {
        // `2` must NOT match `28-…`, and `1` must NOT match `14…` — a number is a whole chapter, not a prefix.
        assert!(resolve_stems("2", &sample()).is_err());
        assert!(resolve_stems("1", &sample()).is_err());
    }

    #[test]
    fn resolve_stems_unknown_input_errs() {
        assert!(resolve_stems("nope", &sample()).is_err());
        assert!(resolve_stems("14b", &sample()).is_err()); // a label prefix, not a full stem nor a bare number
        assert!(resolve_stems("99", &sample()).is_err()); // no such chapter
        assert!(resolve_stems("", &sample()).is_err());
    }

    #[test]
    fn stem_in_chapter_boundaries() {
        assert!(stem_in_chapter("28-wit-abi-boundary", "28"));
        assert!(stem_in_chapter("14b-effects-and-handlers", "14"));
        assert!(!stem_in_chapter("140-foo", "14")); // trailing digit — different chapter number
        assert!(!stem_in_chapter("28-x", "2")); // `2` is not chapter `28`
    }

    #[test]
    fn a_shared_execution_baseline_edit_is_a_flip() {
        // The canonical flip signal: a `todo`/`decline` graded `pass` rewrites one of these three.
        for f in [
            "spec/semantics/.gate-baseline",
            "spec/semantics/.gate-baseline-rust",
            "spec/semantics/.gate-baseline-rust-async",
        ] {
            assert!(
                is_decline_flip_diff(&[f.to_string()]),
                "{f} should trip the flip guard"
            );
        }
    }

    #[test]
    fn a_backend_mod_predicate_edit_is_a_flip() {
        for f in [
            "implementation/seed/crates/rcdzc/src/backend/rust/mod.rs",
            "implementation/seed/crates/rcdzc/src/backend/wasm/mod.rs",
        ] {
            assert!(
                is_decline_flip_diff(&[f.to_string()]),
                "{f} should trip the flip guard"
            );
        }
    }

    #[test]
    fn a_non_flip_diff_does_not_trip_the_guard() {
        // The quote-gate baseline is a syntactic round-trip, not an execution outcome; a corpus `.sexp`
        // edit, a doc, and a deeper backend file are not the agreed `backend/<x>/mod.rs` predicate.
        let changed = [
            "spec/semantics/.quote-gate-baseline".to_string(),
            "spec/semantics/21-host-closures.sexp".to_string(),
            "commands/gate.md".to_string(),
            "implementation/seed/crates/rcdzc/src/backend/wasm/envelope/mod.rs".to_string(),
            "implementation/seed/crates/rcdzc/src/lib.rs".to_string(),
        ];
        assert!(
            !is_decline_flip_diff(&changed),
            "no path here is a shared baseline or a backend `<x>/mod.rs`: {changed:?}"
        );
    }

    #[test]
    fn empty_diff_is_not_a_flip() {
        assert!(!is_decline_flip_diff(&[]));
    }

    #[test]
    fn decide_flag_precedence_and_auto() {
        let flip = ["spec/semantics/.gate-baseline".to_string()];
        let non_flip = ["commands/gate.md".to_string()];
        // --with-lib-tests wins over everything, including --no-lib-tests.
        assert_eq!(
            decide_lib_tests(true, true, Some(&flip)),
            LibTestDecision::ForcedOn
        );
        assert_eq!(
            decide_lib_tests(true, false, None),
            LibTestDecision::ForcedOn
        );
        // --no-lib-tests forces off even on a flip-shaped diff.
        assert_eq!(
            decide_lib_tests(false, true, Some(&flip)),
            LibTestDecision::ForcedOff
        );
        // Neither flag: auto-detect from the diff.
        assert_eq!(
            decide_lib_tests(false, false, Some(&flip)),
            LibTestDecision::AutoFlip
        );
        assert_eq!(
            decide_lib_tests(false, false, Some(&non_flip)),
            LibTestDecision::AutoNoFlip
        );
        // Un-inspectable diff fails SAFE (runs the guard).
        assert_eq!(
            decide_lib_tests(false, false, None),
            LibTestDecision::FailSafe
        );
    }

    #[test]
    fn only_auto_no_flip_and_forced_off_skip_the_lib_tests() {
        assert!(!LibTestDecision::ForcedOff.runs_lib_tests());
        assert!(!LibTestDecision::AutoNoFlip.runs_lib_tests());
        assert!(LibTestDecision::ForcedOn.runs_lib_tests());
        assert!(LibTestDecision::AutoFlip.runs_lib_tests());
        assert!(LibTestDecision::FailSafe.runs_lib_tests());
    }
}
