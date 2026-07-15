//! `cargo xtask duvet-check` — a citation-coverage regression gate.
//!
//! The conformance corpus is protected by EXECUTION (the behavior gate), but the requirement→code
//! CITATION coverage (the `//=` / `//#` duvet annotations that trace each RFC-2119 requirement to the
//! code that implements it) was UNGATED: any peer could delete a citation, or a spec reword could
//! strand one, and no check caught the coverage drop. This gate closes that hole.
//!
//! WHY NOT commit `.duvet/snapshot.txt`: that file is deliberately gitignored — its tags come from the
//! per-build compiler and it churns on every regeneration, so it is not a durable, shareable baseline
//! (committing it would make `xtask check` red on any other machine). Instead we gate on a DERIVED,
//! machine-STABLE metric: the COUNT of citation annotations duvet extracts (`duvet report --json`).
//! The citation count is a spec+source identity (it drops by exactly one when a `//=` is deleted), and
//! `implementation/` is tracked in git, so the count reproduces on any clone. We commit a small
//! `.duvet/coverage-floor.json` `{ "cited": N, "total": M }` and FAIL if live `cited` falls below the
//! committed floor — a citation LOSS turns `check` red, which is the whole point, without pinning the
//! churny snapshot. `v-duvet-coverage` owns GROWING the number: when it adds citations it bumps the
//! floor with `xtask duvet-check --save`.
//!
//! FAIL-SOFT — but ONLY for a genuinely-absent tool. If the `duvet` binary is not installed, this
//! SKIPS with a note (the gate must never turn `check` red merely because a machine lacks the optional
//! tool). But if duvet IS installed and `duvet report` FAILS (the common cause: a stranded citation —
//! a `//=` / `//#` pointing at spec text that was reworded/removed), that is NOT a clean skip: it means
//! the gate can't run, which is exactly the regression it exists to catch, so it FAILS loudly. (An
//! earlier version conflated the two and silently disabled itself when a stranded citation broke the
//! report — green-by-skip. The `Measurement` enum below keeps `Absent` and `ReportFailed` distinct.)

use std::path::Path;
use std::process::Command;

use crate::Paths;

/// The committed floor file, relative to the repo root.
const FLOOR_REL: &str = ".duvet/coverage-floor.json";

/// The machine-stable coverage counts we gate on. `cited` is the count of citation annotations (the
/// `//=` / `//#` traces from code to a requirement); `total` is the count of extracted SPEC
/// requirements. Only `cited` is enforced as a floor (regression = citation loss); `total` is recorded
/// for context (it moves when the spec text changes, which is a legitimate spec edit, not a regression).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Coverage {
    cited: u64,
    total: u64,
}

/// Entry point. `save` records the current counts as the new floor (what `v-duvet-coverage` runs after
/// adding citations); otherwise this enforces the committed floor and exits non-zero on a regression.
pub fn run(paths: &Paths, save: bool) {
    let live = match measure(paths) {
        Measurement::Ok(cov) => cov,
        Measurement::Absent => {
            // The `duvet` binary is genuinely not installed → fail-soft skip (never red on a machine
            // lacking the optional tool). This is the ONLY legitimate skip.
            println!(
                "duvet-check: `duvet` is not installed — SKIPPING the citation gate. \
                 Install duvet to enforce it locally; this is not a failure."
            );
            return;
        }
        Measurement::ReportFailed(why) => {
            // duvet IS installed but `duvet report` failed — almost always a STRANDED citation (a
            // `//=` / `//#` pointing at spec text that no longer exists). This previously fell into the
            // skip path, silently DISABLING the gate (it was green-by-skip on trunk for a while). That
            // is exactly the regression the gate exists to catch, so it must be LOUD, not a skip: FAIL.
            eprintln!(
                "duvet-check: `duvet report` FAILED — the citation gate could NOT run. This is NOT a \
                 clean skip: duvet is installed but erroring, which is almost always a STRANDED \
                 citation (a //= / //# whose spec sentence was reworded/removed). Fix the citation so \
                 `duvet report` succeeds again.\n  duvet error: {why}"
            );
            std::process::exit(1);
        }
    };

    let floor_path = paths.repo.join(FLOOR_REL);
    if save {
        write_floor(&floor_path, live);
        println!(
            "duvet-check: saved floor = {{ cited: {}, total: {} }} to {}",
            live.cited, live.total, FLOOR_REL
        );
        return;
    }

    let Some(floor) = read_floor(&floor_path) else {
        // No committed floor yet → nothing to enforce. Tell the caller how to create one, but don't
        // fail (a fresh tree without the floor shouldn't be red).
        println!(
            "duvet-check: no committed floor at {FLOOR_REL} (live: cited={}, total={}). \
             Create it with `cargo xtask duvet-check --save`.",
            live.cited, live.total
        );
        return;
    };

    if live.cited < floor.cited {
        eprintln!(
            "duvet-check: CITATION COVERAGE REGRESSED — live cited={} < floor cited={} \
             (a //= / //# citation was deleted or stranded by a spec reword).\n  \
             Restore the citation, or — if the drop is intentional — lower the floor with \
             `cargo xtask duvet-check --save` and explain why in the commit.\n  \
             (total requirements: live={}, floor={})",
            live.cited, floor.cited, live.total, floor.total
        );
        std::process::exit(1);
    }

    if live.cited > floor.cited {
        // Coverage GREW above the floor — not a failure, but the floor should be bumped so the new
        // coverage is protected. Nudge (don't fail): v-duvet-coverage bumps it when it lands citations.
        println!(
            "duvet-check: OK — cited={} ≥ floor={} (coverage grew above the floor; \
             bump it with `--save` to protect the {} new citation(s)). total={}.",
            live.cited,
            floor.cited,
            live.cited - floor.cited,
            live.total
        );
    } else {
        println!(
            "duvet-check: OK — cited={} meets floor={} (total requirements={}).",
            live.cited, floor.cited, live.total
        );
    }
}

/// The outcome of trying to measure coverage — distinguished so `run` can SKIP on a genuinely-absent
/// tool but FAIL LOUDLY when duvet is present yet erroring (a stranded citation), instead of silently
/// disabling the gate for both.
enum Measurement {
    /// duvet ran and its report parsed → the coverage counts.
    Ok(Coverage),
    /// The `duvet` binary is not installed (command not found) → the only legitimate skip.
    Absent,
    /// duvet is installed but `duvet report` failed / its output couldn't be parsed → must be loud.
    ReportFailed(String),
}

/// Run `duvet report --json <tmp>` and count the citation + spec annotations, distinguishing a missing
/// binary (→ `Absent`) from a present-but-failing one (→ `ReportFailed`).
fn measure(paths: &Paths) -> Measurement {
    // Emit the JSON to a temp path under the repo's target dir (never committed).
    let out = paths.repo.join("target/duvet-report.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let output = Command::new("duvet")
        .current_dir(&paths.repo)
        .args(["report", "--json"])
        .arg(&out)
        .output();
    match output {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Measurement::Absent,
        Err(e) => Measurement::ReportFailed(format!("could not run duvet: {e}")),
        Ok(o) if !o.status.success() => {
            // duvet ran but exited non-zero — surface its stderr (it names the stranded citation).
            let stderr = String::from_utf8_lossy(&o.stderr);
            let tail: String = stderr
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            Measurement::ReportFailed(if tail.is_empty() {
                format!("duvet report exited with {}", o.status)
            } else {
                tail
            })
        }
        Ok(_) => match std::fs::read_to_string(&out) {
            Err(e) => Measurement::ReportFailed(format!("could not read duvet report: {e}")),
            Ok(text) => match count_annotations(&text) {
                Some(cov) => Measurement::Ok(cov),
                None => Measurement::ReportFailed(
                    "duvet report JSON had no `annotations` array (unexpected shape)".to_string(),
                ),
            },
        },
    }
}

/// Count citation vs SPEC annotations in a `duvet report --json` document. A SPEC-typed annotation is
/// an extracted requirement (the denominator); an annotation with NO `type` is a CITATION (a `//=` /
/// `//#` from source pointing at a requirement) — the numerator that drops when a citation is removed.
/// Kept as a free function so it is unit-testable without invoking duvet.
fn count_annotations(json: &str) -> Option<Coverage> {
    let doc: serde_json::Value = serde_json::from_str(json).ok()?;
    let anns = doc.get("annotations")?.as_array()?;
    let mut cited = 0u64;
    let mut total = 0u64;
    for a in anns {
        match a.get("type").and_then(|t| t.as_str()) {
            Some("SPEC") => total += 1,
            // No `type` field → a citation annotation (source→requirement trace).
            None => cited += 1,
            // Any other typed annotation is neither a requirement nor a citation for our metric.
            Some(_) => {}
        }
    }
    Some(Coverage { cited, total })
}

/// Read the committed floor, or `None` if it is absent/malformed.
fn read_floor(path: &Path) -> Option<Coverage> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(Coverage {
        cited: doc.get("cited")?.as_u64()?,
        total: doc.get("total")?.as_u64().unwrap_or(0),
    })
}

/// Write the floor file, pretty and with a `//`-style note (JSON has no comments, so use a `_note` key)
/// so a reader knows what it is and who bumps it.
fn write_floor(path: &Path, cov: Coverage) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let doc = format!(
        "{{\n  \"_note\": \"Citation-coverage floor for `cargo xtask duvet-check` (wired into \
         `xtask check`). `cited` = number of //= / //# duvet citation annotations; the gate FAILS if \
         live cited drops below it (a deleted/stranded citation). v-duvet-coverage bumps this with \
         `xtask duvet-check --save` when it adds citations. NOT the churny .duvet/snapshot.txt — this \
         is a machine-stable count.\",\n  \"cited\": {},\n  \"total\": {}\n}}\n",
        cov.cited, cov.total
    );
    std::fs::write(path, doc).expect("write coverage-floor.json");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_spec_as_total_and_untyped_as_cited() {
        // Two SPEC requirements, one citation (no `type`), one other-typed (ignored).
        let json = r#"{"annotations":[
            {"type":"SPEC","target_path":"a.md"},
            {"type":"SPEC","target_path":"b.md"},
            {"source":"implementation/x.rs","target_path":"a.md"},
            {"type":"TEST","target_path":"a.md"}
        ]}"#;
        let c = count_annotations(json).unwrap();
        assert_eq!(c.total, 2);
        assert_eq!(c.cited, 1);
    }

    #[test]
    fn empty_annotations_is_zero_zero() {
        let c = count_annotations(r#"{"annotations":[]}"#).unwrap();
        assert_eq!(c.cited, 0);
        assert_eq!(c.total, 0);
    }

    #[test]
    fn malformed_json_is_none() {
        assert!(count_annotations("not json").is_none());
        // Missing the annotations array → None (fail-soft, not a crash).
        assert!(count_annotations(r#"{"other":1}"#).is_none());
    }
}
