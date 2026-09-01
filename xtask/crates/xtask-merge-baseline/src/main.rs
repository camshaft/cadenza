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
//! Carved out of `xtask/src/main.rs`'s `Cmd::MergeBaseline` (v-xtask-decompose, seq-202 shrink). Not a
//! nix-app: git needs a fast local binary, not a per-merge nix eval — the driver points at this crate's
//! binary. Deps `xtask-support` only.

use std::path::Path;
use std::process::ExitCode;

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
    /// Clean union — `ours` was overwritten in place with the sorted+deduped union.
    Merged,
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
            if std::fs::write(ours, &merged).is_err() {
                return Outcome::IoError;
            }
            Outcome::Merged
        }
        Err(BaselineMergeErr::Conflict(titles)) => Outcome::Conflict(titles),
        Err(BaselineMergeErr::Unparseable) => Outcome::Unparseable,
    }
}

fn merge(ours: &Path, theirs: &Path) -> ExitCode {
    match merge_files(ours, theirs) {
        Outcome::Merged => {
            eprintln!(
                "merge-baseline: resolved {} baseline merge (union + verdict-aware dedup)",
                ours.display()
            );
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
        assert_eq!(merge_files(&ours, &theirs), Outcome::Merged);
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
}
