//! `xtask-save-baseline` — regenerate a `.gate-baseline*` file from a harvested per-case verdict stream.
//!
//! The seq-202 gate-command delete moves the heavy `gate --save` grading to nix: the nix corpus checks
//! already grade every case, so a `cdz-run --emit-verdict` CLASSIFY mode emits each case's `tag<TAB>
//! description` (verdict tag then a tab then the case description — no header; the classify runs the grade,
//! maps `Grade`→`Verdict`, and always exits 0), a `.#corpus-verdicts` derivation harvests every case's
//! line into one file (content-addressed, cached), and `apps.save-baseline` builds that then runs THIS
//! leaf. This tool is the thin write half: read the harvested verdict lines and (re)write the baseline via
//! `xtask_support::serialize_baseline` — no corpus re-run, no compiler pipeline.
//!
//! Usage: `xtask-save-baseline <verdicts-file> <baseline-out>`
//!   - `<verdicts-file>`: the harvested stream, one `tag<TAB>description` line per case (`# …` comment /
//!     blank lines are skipped, so a stream that happens to carry a header is tolerated).
//!   - `<baseline-out>`: the `.gate-baseline*` path to write (overwritten).
//!
//! The vocabulary + format are owned by v-corpus-harness: `Verdict` is exactly `pass`/`todo`/`fail`, and
//! `serialize_baseline` sorts, adds the `#` header, and writes tab-separated `tag<TAB>description` — so
//! feeding a `BTreeMap<description, Verdict>` back through it round-trips byte-identically (via
//! `Verdict::parse`) with what `check_baseline` reads.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use xtask_support::{Verdict, serialize_baseline};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [verdicts, baseline_out] = args.as_slice() else {
        eprintln!("usage: xtask-save-baseline <verdicts-file> <baseline-out>");
        return ExitCode::from(2);
    };
    match save(Path::new(verdicts), Path::new(baseline_out)) {
        Outcome::Wrote(n) => {
            println!("xtask-save-baseline: wrote {n} case verdicts → {baseline_out}");
            ExitCode::SUCCESS
        }
        Outcome::Unreadable(e) => {
            eprintln!("xtask-save-baseline: reading verdicts {verdicts}: {e}");
            ExitCode::from(1)
        }
        Outcome::BadVerdict(bad) => {
            eprintln!(
                "xtask-save-baseline: {bad} — the verdict stream must be `tag<TAB>description` lines \
                 with tag ∈ pass/todo/fail (the classify emitter is out of sync with the baseline vocab)."
            );
            ExitCode::from(1)
        }
        Outcome::WriteFailed(e) => {
            eprintln!("xtask-save-baseline: writing baseline {baseline_out}: {e}");
            ExitCode::from(1)
        }
    }
}

/// What the save did — the testable decision, separated from the process-exit + diagnostic layer.
/// The safety invariant the tests pin: `out` is written ONLY on a fully-parsed stream (`Wrote`); an
/// `Unreadable` source or a `BadVerdict` line returns BEFORE the write, so a malformed classify
/// emitter can never silently corrupt (or half-write) the committed baseline. `Debug`/`PartialEq`
/// so a test can assert on it (unlike a bare `ExitCode`).
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Parsed N case verdicts and (over)wrote `out` with their serialized baseline.
    Wrote(usize),
    /// The verdicts source couldn't be read — `out` untouched.
    Unreadable(String),
    /// A stream line isn't `tag<TAB>description` with a known verdict tag — `out` untouched.
    BadVerdict(String),
    /// The baseline couldn't be written to `out`.
    WriteFailed(String),
}

/// Read the harvested verdict stream, parse it, and (only on a fully-valid stream) overwrite `out`
/// with the serialized baseline. Every error path returns before the write so `out` is never
/// half-written or corrupted from a malformed source — that no-write-on-error invariant is what the
/// file-level tests pin.
fn save(verdicts: &Path, out: &Path) -> Outcome {
    let text = match std::fs::read_to_string(verdicts) {
        Ok(t) => t,
        Err(e) => return Outcome::Unreadable(e.to_string()),
    };
    let by_desc = match parse_verdicts(&text) {
        Ok(m) => m,
        Err(bad) => return Outcome::BadVerdict(bad),
    };
    let body = serialize_baseline(&by_desc);
    if let Err(e) = std::fs::write(out, &body) {
        return Outcome::WriteFailed(e.to_string());
    }
    Outcome::Wrote(by_desc.len())
}

/// Parse the harvested `tag<TAB>description` verdict stream into a `description → Verdict` map (de-duping
/// by description, last-wins, matching `serialize_baseline`/`check_baseline`'s map-load). `#`-comment and
/// blank lines are skipped (a stream carrying a header is tolerated). Returns `Err(<line desc>)` on the
/// first line that isn't `tag<TAB>description` with a known verdict tag — a hard fail, since a malformed
/// verdict would silently corrupt the committed baseline.
fn parse_verdicts(text: &str) -> Result<BTreeMap<String, Verdict>, String> {
    let mut by_desc = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((tag, desc)) = line.split_once('\t') else {
            return Err(format!("line {} has no tab separator: {line:?}", i + 1));
        };
        let Some(verdict) = Verdict::parse(tag) else {
            return Err(format!("line {} has an unknown verdict tag {tag:?}", i + 1));
        };
        by_desc.insert(desc.to_string(), verdict);
    }
    Ok(by_desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `Verdict` derives neither `Debug` nor `PartialEq` (it's v-corpus-harness's crate), so assert via
    // `.tag()` (a `&'static str`) and via `serialize_baseline` output (a `String`) rather than on `Verdict`.
    #[test]
    fn parses_the_three_tags_and_skips_header_and_blanks() {
        let stream = "# gate baseline harvest\n\
                      pass\tadds two ints\n\
                      todo\tdeclines a generic tie\n\
                      fail\tregression-pinned case\n\
                      \n";
        let m = parse_verdicts(stream).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m["adds two ints"].tag(), "pass");
        assert_eq!(m["declines a generic tie"].tag(), "todo");
        assert_eq!(m["regression-pinned case"].tag(), "fail");
    }

    #[test]
    fn round_trips_through_serialize_baseline() {
        // A harvested stream → parse → serialize must be re-parseable to a map that re-serializes
        // byte-identically — the property the whole harvest→leaf→baseline pipeline relies on.
        let stream = "todo\tzeta case\npass\talpha case\n";
        let m = parse_verdicts(stream).unwrap();
        let serialized = serialize_baseline(&m);
        // serialize_baseline sorts by `tag<TAB>desc`; re-parsing (skipping its `#` header) round-trips.
        let reparsed = parse_verdicts(&serialized).unwrap();
        assert_eq!(serialize_baseline(&reparsed), serialized);
    }

    #[test]
    fn last_wins_on_duplicate_description() {
        let m = parse_verdicts("pass\tdup\ntodo\tdup\n").unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m["dup"].tag(), "todo"); // last wins, matching the baseline map-load
    }

    #[test]
    fn rejects_an_unknown_tag() {
        assert!(parse_verdicts("decline\tsome case\n").is_err());
        assert!(parse_verdicts("no-tab-here\n").is_err());
    }

    // --- file-level `save()` tests: the I/O + no-write-on-error contract ---

    use xtask_support::TmpDir;

    #[test]
    fn save_writes_the_serialized_baseline_for_a_valid_stream() {
        let d = TmpDir::new("xtask-save-baseline-test-");
        let verdicts = d.write("verdicts", "todo\tzeta case\npass\talpha case\n");
        let out = d.path("baseline");
        assert_eq!(save(&verdicts, &out), Outcome::Wrote(2));
        // The written file is exactly serialize_baseline of the parsed stream (sorted + `#` header).
        let want =
            serialize_baseline(&parse_verdicts("todo\tzeta case\npass\talpha case\n").unwrap());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), want);
    }

    #[test]
    fn a_bad_verdict_does_not_create_or_touch_the_baseline() {
        // The doc-comment safety invariant: a malformed classify emitter must NOT corrupt the baseline.
        let d = TmpDir::new("xtask-save-baseline-test-");
        let verdicts = d.write("verdicts", "pass\tok case\ndecline\tbad tag\n");
        let out = d.path("baseline");
        assert!(matches!(save(&verdicts, &out), Outcome::BadVerdict(_)));
        // out must not have been created — the write never ran.
        assert!(
            !out.exists(),
            "baseline must not be written on a bad verdict"
        );
    }

    #[test]
    fn a_bad_verdict_leaves_a_preexisting_baseline_untouched() {
        // If a stale baseline already exists, a bad harvest must leave it byte-for-byte intact rather
        // than half-overwriting it.
        let d = TmpDir::new("xtask-save-baseline-test-");
        let verdicts = d.write("verdicts", "pass\tok case\nno-tab-line\n");
        let original = "# gate baseline harvest\npass\tpre-existing\n";
        let out = d.write("baseline", original);
        assert!(matches!(save(&verdicts, &out), Outcome::BadVerdict(_)));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), original);
    }

    #[test]
    fn an_unreadable_verdicts_source_is_an_error_not_a_silent_empty_baseline() {
        let d = TmpDir::new("xtask-save-baseline-test-");
        let missing = d.path("does-not-exist");
        let out = d.path("baseline");
        assert!(matches!(save(&missing, &out), Outcome::Unreadable(_)));
        assert!(!out.exists(), "no baseline on an unreadable source");
    }
}
