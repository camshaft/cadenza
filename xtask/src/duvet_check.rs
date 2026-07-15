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
//! FAIL-SOFT: if the `duvet` binary is not installed, this SKIPS with a note rather than failing — the
//! gate must never turn `check` red merely because a machine lacks the optional tool (that would be the
//! very machine-specific flakiness the committed-snapshot approach was rejected for). A machine that
//! has duvet enforces the floor; one that doesn't, doesn't.

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
    let Some(live) = measure(paths) else {
        // duvet not installed / report failed → fail-soft skip (never red on a missing optional tool).
        println!(
            "duvet-check: `duvet` not available (or report failed) — SKIPPING the citation gate. \
             Install duvet to enforce it locally; this is not a failure."
        );
        return;
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

/// Run `duvet report --json <tmp>` and count the citation + spec annotations. Returns `None` if duvet
/// is missing or the report can't be produced/parsed (→ fail-soft skip).
fn measure(paths: &Paths) -> Option<Coverage> {
    // Emit the JSON to a temp path under the repo's target dir (never committed).
    let out = paths.repo.join("target/duvet-report.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let status = Command::new("duvet")
        .current_dir(&paths.repo)
        .args(["report", "--json"])
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {}
        _ => return None, // duvet missing or errored → skip
    }
    let text = std::fs::read_to_string(&out).ok()?;
    count_annotations(&text)
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
