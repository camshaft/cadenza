//! `xtask-merge-baseline` — the git merge driver for `.gate-baseline*` files.
//!
//! git invokes this per-merge on a `.gate-baseline*` conflict (wired via `.gitattributes`
//! `merge=fleet-baseline` + registered by `fleet up`'s `register_merge_drivers`). It unions the two
//! sides then VERDICT-AWARE DEDUPS (the same [`xtask_support::merge_baseline_union`] the canonicalizer
//! uses): read `ours` (git's `%A`, the merge destination) + `theirs` (`%B`), concatenate, and OVERWRITE
//! `ours` with the sorted + deduped union — collapsing a benign same-verdict duplicate. FAILS (exit 1,
//! leaving the conflict for a human) on a same-description/DIFFERENT-verdict conflict (never silently
//! picks a verdict), and fail-safe on an unreadable side or an unmodelable line.
//!
//! Usage: `xtask-merge-baseline <ours> <theirs>` — git's `%A` (ours/dest, rewritten in place) and `%B`
//! (theirs). git's `%O` (ancestor) is not needed by the union merge, so it is not passed.
//!
//! ORPHAN PRUNE (v-fleet-tooling ↔ v-corpus-harness, #7646): after a clean union it also DROPS any
//! `verdict<TAB>title` line whose title is not a real corpus case — a VANISHED ORPHAN. The union
//! re-adds a stale OLD-title line after a case RETITLE (agent B's side still carries it from the
//! ancestor; union keeps it) → the gate's vanished-check reds. Pruning HERE fixes it at the exact
//! re-add point, in ALL merge paths (merge/rebase/cherry-pick — the last is what `fleet sync` does, and
//! is why a pre-commit hook can't catch it: the orphan lands in an already-committed replay). The
//! corpus title set comes from the tiny fail-open `corpus_case_titles` (AST-based); FAIL-OPEN is
//! load-bearing: if it can't read the WHOLE corpus it returns `None` and we SKIP the prune (write the
//! union untouched) — a prune that can't see the corpus must NEVER strip. The gate vanished-check stays
//! the backstop.
//!
//! Carved out of `xtask/src/main.rs`'s `Cmd::MergeBaseline` (v-xtask-decompose, seq-202 shrink). Not a
//! nix-app: git needs a fast local binary, not a per-merge nix eval — the driver points at this crate's
//! binary. Deps `xtask-support` + the tiny `corpus-case-titles` leaf (no clap/cdz/compiler/nix).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use corpus_case_titles::corpus_case_titles;
use xtask_support::{BaselineMergeErr, merge_baseline_union};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [ours, theirs] = args.as_slice() else {
        eprintln!("usage: xtask-merge-baseline <ours> <theirs>  (git %A + %B)");
        return ExitCode::from(2);
    };
    merge(Path::new(ours), Path::new(theirs))
}

/// What the merge did — the testable decision, separated from the process-exit + diagnostic layer so
/// the I/O contract (union rewrites `ours`; a REFUSAL leaves `ours` byte-for-byte untouched for the
/// human) can be unit-tested without asserting on a non-`PartialEq` `ExitCode`.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Clean union — `ours` was overwritten in place with the sorted+deduped union, then any
    /// vanished-orphan lines (title with no corpus case) were pruned; `pruned` lists what was dropped.
    Merged { pruned: Vec<String> },
    /// A same-description/different-verdict conflict (carries the conflicting titles). `ours` untouched.
    Conflict(Vec<String>),
    /// A baseline line isn't `verdict<TAB>description`. `ours` untouched.
    Unparseable,
    /// A side couldn't be read, or the merged union couldn't be written back. `ours` may be untouched.
    IoError,
}

/// Do the merge and (only on a clean union) rewrite `ours` in place. Every REFUSAL path (`Conflict`,
/// `Unparseable`, `IoError`) leaves `ours` unwritten so git keeps the conflict for a human — that
/// no-write-on-refusal invariant is the leaf's whole point and is what the tests pin.
fn merge_files(ours: &Path, theirs: &Path) -> Outcome {
    let read = |p: &Path| -> Option<String> { std::fs::read_to_string(p).ok() };
    let (Some(o), Some(t)) = (read(ours), read(theirs)) else {
        return Outcome::IoError;
    };
    match merge_baseline_union(&o, &t) {
        Ok(merged) => {
            // Prune vanished orphans the union may have re-added, against the REAL corpus case titles.
            // FAIL-OPEN: `corpus_case_titles` returns `None` (empty/unreadable/unparseable/zero-case) →
            // skip the prune and write the union untouched; the gate vanished-check is the backstop.
            let corpus = corpus_sexps();
            let (to_write, pruned) = match corpus_case_titles(&corpus) {
                Some(titles) => prune_orphan_lines(&merged, &titles),
                None => (merged, Vec::new()),
            };
            if std::fs::write(ours, &to_write).is_err() {
                return Outcome::IoError;
            }
            Outcome::Merged { pruned }
        }
        Err(BaselineMergeErr::Conflict(titles)) => Outcome::Conflict(titles),
        Err(BaselineMergeErr::Unparseable) => Outcome::Unparseable,
    }
}

/// The corpus s-expression files (`spec/semantics/*.sexp`) relative to the CWD. git runs a merge driver
/// with cwd at the worktree root, so the corpus is here. Returns an EMPTY vec on any dir-read error →
/// `corpus_case_titles` then returns `None` → the caller SKIPS the prune (fail-open: a prune that can't
/// see the corpus must never strip). Sorted for a deterministic read order.
fn corpus_sexps() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(Path::new("spec/semantics")) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sexp"))
        .collect();
    v.sort();
    v
}

/// Drop the `verdict<TAB>title` lines whose title is not a real corpus case (a vanished orphan the union
/// re-added). Comment (`#…`) and blank lines are preserved verbatim — only real data lines are pruned,
/// mirroring [`merge_baseline_union`]'s own line model (skip `#`/blank, else split on the first TAB).
/// Returns the pruned text (re-terminated with a trailing newline like `serialize_baseline`) + the
/// titles dropped (for the diagnostic + tests). Pure — the whole prune decision is here, testable
/// without the corpus I/O.
fn prune_orphan_lines(merged: &str, titles: &BTreeSet<String>) -> (String, Vec<String>) {
    let mut dropped = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for line in merged.lines() {
        if line.starts_with('#') || line.is_empty() {
            kept.push(line);
            continue;
        }
        match line.split_once('\t') {
            // A data line whose title has no corpus case → orphan, drop it.
            Some((_verdict, title)) if !titles.contains(title) => dropped.push(title.to_string()),
            // A real title, or a line we don't model as data → keep verbatim.
            _ => kept.push(line),
        }
    }
    let mut out = kept.join("\n");
    out.push('\n'); // match serialize_baseline's trailing newline
    (out, dropped)
}

fn merge(ours: &Path, theirs: &Path) -> ExitCode {
    match merge_files(ours, theirs) {
        Outcome::Merged { pruned } => {
            eprintln!(
                "merge-baseline: resolved {} baseline merge (union + verdict-aware dedup)",
                ours.display()
            );
            if !pruned.is_empty() {
                eprintln!(
                    "merge-baseline: pruned {} vanished-orphan line(s) (title has no corpus case; \
                     union re-added a stale old-title after a retitle):\n    {}",
                    pruned.len(),
                    pruned.join("\n    ")
                );
            }
            ExitCode::SUCCESS
        }
        Outcome::Conflict(titles) => {
            eprintln!(
                "merge-baseline: REFUSING {} — {} same-title/DIFFERENT-verdict conflict(s) can't be \
                 auto-merged (would mask one via last-wins); leaving the conflict for a human:\n    {}",
                ours.display(),
                titles.len(),
                titles.join("\n    ")
            );
            ExitCode::FAILURE
        }
        Outcome::Unparseable => {
            eprintln!(
                "merge-baseline: REFUSING {} — a baseline line isn't `verdict<TAB>description`; won't \
                 rewrite data I can't model. Leaving the conflict for a human.",
                ours.display()
            );
            ExitCode::FAILURE
        }
        Outcome::IoError => {
            eprintln!(
                "merge-baseline: could not read/write baseline sides ({} / {}) — leaving the conflict for a human",
                ours.display(),
                theirs.display()
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xtask_support::TmpDir;

    #[test]
    fn clean_union_rewrites_ours_in_place_with_the_deduped_union() {
        let d = TmpDir::new("xtask-merge-baseline-test-");
        let ours = d.write("ours", "pass\talpha\n");
        let theirs = d.write("theirs", "todo\tzeta\n");
        // Runs with cwd at the crate dir (no spec/semantics/ here), so corpus_case_titles → None →
        // the prune is skipped (fail-open) and nothing is dropped.
        assert!(matches!(
            merge_files(&ours, &theirs),
            Outcome::Merged { ref pruned } if pruned.is_empty()
        ));
        // `ours` now holds the sorted union of both sides (theirs merged in), not its original content.
        let written = std::fs::read_to_string(&ours).unwrap();
        assert!(written.contains("pass\talpha"), "kept ours: {written:?}");
        assert!(
            written.contains("todo\tzeta"),
            "merged theirs in: {written:?}"
        );
    }

    #[test]
    fn a_conflict_refuses_and_leaves_ours_byte_for_byte_untouched() {
        // Same description, different verdict — the union can't pick one without masking the other.
        let d = TmpDir::new("xtask-merge-baseline-test-");
        let original = "pass\tsame case\n";
        let ours = d.write("ours", original);
        let theirs = d.write("theirs", "todo\tsame case\n");
        assert_eq!(
            merge_files(&ours, &theirs),
            Outcome::Conflict(vec!["same case".to_string()])
        );
        // The whole point: a refusal must NOT rewrite `ours` — git keeps the conflict for a human.
        assert_eq!(std::fs::read_to_string(&ours).unwrap(), original);
    }

    #[test]
    fn an_unparseable_line_refuses_and_leaves_ours_untouched() {
        let d = TmpDir::new("xtask-merge-baseline-test-");
        let original = "pass\tok case\nthis-line-has-no-tab\n";
        let ours = d.write("ours", original);
        let theirs = d.write("theirs", "todo\tother\n");
        assert_eq!(merge_files(&ours, &theirs), Outcome::Unparseable);
        assert_eq!(std::fs::read_to_string(&ours).unwrap(), original);
    }

    #[test]
    fn an_unreadable_side_is_an_io_error_not_a_silent_success() {
        let d = TmpDir::new("xtask-merge-baseline-test-");
        let ours = d.write("ours", "pass\talpha\n");
        let missing = d.path("does-not-exist");
        assert_eq!(merge_files(&ours, &missing), Outcome::IoError);
    }

    fn titleset(ts: &[&str]) -> BTreeSet<String> {
        ts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prune_drops_only_the_orphan_data_line_keeping_real_titles_and_the_header() {
        // A retitle left `old title` in the baseline with no corpus case; `alpha`/`zeta` are real.
        let merged = "# gate baseline — hdr\npass\talpha\nfail\told title\ntodo\tzeta\n";
        let (out, dropped) = prune_orphan_lines(merged, &titleset(&["alpha", "zeta"]));
        assert_eq!(dropped, vec!["old title".to_string()]);
        assert_eq!(out, "# gate baseline — hdr\npass\talpha\ntodo\tzeta\n");
        // header preserved, orphan gone, trailing newline preserved.
        assert!(out.starts_with("# gate baseline"));
        assert!(!out.contains("old title"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn prune_is_a_noop_when_every_title_has_a_case() {
        let merged = "# hdr\npass\talpha\ntodo\tzeta\n";
        let (out, dropped) = prune_orphan_lines(merged, &titleset(&["alpha", "zeta"]));
        assert!(dropped.is_empty(), "nothing to prune");
        assert_eq!(out, merged, "clean baseline is left byte-identical");
    }

    #[test]
    fn prune_never_touches_comment_or_blank_lines_even_if_all_data_is_orphan() {
        // Comment + blank lines are structural, not data — never pruned (their "title" is no case).
        let merged = "# hdr\n\npass\tghost\n";
        let (out, dropped) = prune_orphan_lines(merged, &titleset(&["real"]));
        assert_eq!(dropped, vec!["ghost".to_string()]);
        // The header + blank line survive; only the orphan data line is removed.
        assert_eq!(out, "# hdr\n\n");
    }

    #[test]
    fn prune_matches_titles_with_embedded_tabs_via_first_tab_split() {
        // split_once('\t') → the title is everything after the FIRST tab (mirrors merge_baseline_union),
        // so a title that itself contains a tab still matches by its full text.
        let merged = "pass\ta\tb\n";
        let (kept_out, kept_dropped) = prune_orphan_lines(merged, &titleset(&["a\tb"]));
        assert!(kept_dropped.is_empty());
        assert_eq!(kept_out, "pass\ta\tb\n");
        let (drop_out, drop_dropped) = prune_orphan_lines(merged, &titleset(&["a"]));
        assert_eq!(drop_dropped, vec!["a\tb".to_string()]);
        assert_eq!(drop_out, "\n"); // only the orphan data line existed → header-less empty body
    }
}
