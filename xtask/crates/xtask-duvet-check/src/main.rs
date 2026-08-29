//! `xtask-duvet-check` — a citation-coverage regression gate.
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
//! Three non-`Ok` outcomes, each handled distinctly (see the `Measurement` enum). `Absent` — the
//! `duvet` binary isn't installed → SKIP (never redden `check` for a missing optional tool).
//! `Stranded` — duvet ran but ABORTED on a stranded citation (a `//=` / `//#` pointing at a spec
//! section that was renamed/removed); `duvet report` emits no JSON, so coverage is unmeasurable, but
//! this is a DISTINCT, self-evident, single-owner fix (repoint one citation), NOT a coverage
//! regression — blocking every agent's `check` on one stale reference is too blunt, so we WARN (naming
//! the exact citation) and SKIP the floor check rather than hard-fail. `ReportFailed` — duvet failed
//! for some OTHER reason (crash / unexpected output) → FAIL loudly.
//!
//! History: v1 conflated absent+failed and silently green-by-skip'd on a stranding; v2 over-corrected
//! and hard-failed the whole fleet's gate on any stranding; this v3 names the stranding + doesn't block.
//!
//! Repo root from `CDZ_REPO_ROOT` (else cwd), matching xtask's `Paths::resolve`. Carved out of
//! `xtask/src/duvet_check.rs` (v-xtask-decompose).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed floor file, relative to the repo root.
const FLOOR_REL: &str = ".duvet/coverage-floor.json";

fn main() {
    // Only flag: `--save` records the current counts as the new floor. Any other/extra arg is usage.
    let save = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [flag] if flag == "--save" => true,
        _ => {
            eprintln!("usage: xtask-duvet-check [--save]");
            std::process::exit(2);
        }
    };

    // Repo root from `CDZ_REPO_ROOT` (the nix-app path passes it); else the current dir (bare cargo run
    // from the repo root), matching xtask's `Paths::resolve`.
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));

    run(&repo, save);
}

/// Entry point. `save` records the current counts as the new floor (what `v-duvet-coverage` runs after
/// adding citations); otherwise this enforces the committed floor and exits non-zero on a regression.
fn run(repo: &Path, save: bool) {
    let live = match measure(repo) {
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
        Measurement::Stranded(loc) => {
            // A STRANDED citation aborted `duvet report` (no JSON → coverage unmeasurable). This is a
            // DISTINCT, self-evident, single-owner problem (repoint one `//=`/`//#` at the renamed
            // section) — NOT a coverage regression, and NOT a reason to redden every agent's
            // `xtask check`. So WARN loudly (naming the exact citation to fix) but do NOT block: the
            // coverage-floor check is simply skipped this run (it can't measure). The owner fixing the
            // citation restores measurement; nobody else is held hostage by one stale reference.
            eprintln!(
                "duvet-check: WARNING — a STRANDED citation aborted `duvet report`, so citation \
                 coverage couldn't be measured this run (gate SKIPPED, not failed). Repoint it at the \
                 current spec section:\n  {loc}\n  (A stranding is a fixable single-citation issue, \
                 not a coverage regression — it doesn't block the gate, but please fix it so coverage \
                 is measured again.)"
            );
            return;
        }
        Measurement::ReportFailed(why) => {
            // duvet is installed and did NOT abort on a stranding, yet still failed (crash / unexpected
            // output). That's genuinely broken and rare — fail loudly so it's investigated.
            eprintln!(
                "duvet-check: `duvet report` FAILED for a non-stranding reason — the citation gate \
                 could NOT run. Investigate:\n  duvet error: {why}"
            );
            std::process::exit(1);
        }
    };

    let floor_path = repo.join(FLOOR_REL);
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

/// The machine-stable coverage counts we gate on. `cited` is the count of citation annotations (the
/// `//=` / `//#` traces from code to a requirement); `total` is the count of extracted SPEC
/// requirements. Only `cited` is enforced as a floor (regression = citation loss); `total` is recorded
/// for context (it moves when the spec text changes, which is a legitimate spec edit, not a regression).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Coverage {
    cited: u64,
    total: u64,
}

/// The outcome of trying to measure coverage. `run` treats each distinctly: SKIP on a genuinely-absent
/// tool; WARN-but-don't-block on a STRANDED citation (a distinct, self-evident, single-owner problem
/// that shouldn't blank every agent's coverage gate); and FAIL only on an OTHERWISE-broken report.
enum Measurement {
    /// duvet ran and its report parsed → the coverage counts.
    Ok(Coverage),
    /// The `duvet` binary is not installed (command not found) → the only legitimate skip.
    Absent,
    /// duvet ran but ABORTED on a stranded citation — a `//=` / `//#` pointing at a spec section that
    /// was renamed/removed. `duvet report` emits no JSON in this case, so coverage can't be measured,
    /// but this is a DISTINCT fixable issue (repoint the citation), not a coverage regression, and it's
    /// self-evidently loud + single-owner — so blocking the whole fleet's gate on it is too blunt.
    /// Carries the human-readable location (`file:line → missing §slug`) parsed from duvet's error.
    Stranded(String),
    /// duvet is installed but `duvet report` failed for another reason (crash / unexpected output) →
    /// genuinely broken, must be loud.
    ReportFailed(String),
}

/// Run `duvet report --json <tmp>` and count the citation + spec annotations, distinguishing a missing
/// binary (→ `Absent`) from a present-but-failing one (→ `ReportFailed`).
fn measure(repo: &Path) -> Measurement {
    // Emit the JSON to a temp path under the repo's target dir (never committed).
    let out = repo.join("target/duvet-report.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let output = Command::new("duvet")
        .current_dir(repo)
        .args(["report", "--json"])
        .arg(&out)
        .output();
    match output {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Measurement::Absent,
        Err(e) => Measurement::ReportFailed(format!("could not run duvet: {e}")),
        Ok(o) if !o.status.success() => {
            // duvet ran but exited non-zero. A STRANDED citation is the common, benign-to-the-gate
            // case: `duvet report` aborts with a `missing section "<slug>"` … `[<file>:<line>]` error.
            // Classify it as `Stranded` (warn, don't block) vs a genuine `ReportFailed` (block).
            let stderr = String::from_utf8_lossy(&o.stderr);
            match parse_stranded(&stderr) {
                Some(loc) => Measurement::Stranded(loc),
                None => {
                    let tail = last_lines(&stderr, 6);
                    Measurement::ReportFailed(if tail.is_empty() {
                        format!("duvet report exited with {}", o.status)
                    } else {
                        tail
                    })
                }
            }
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

/// Parse a duvet `missing section` (stranded-citation) error out of its stderr, returning a compact
/// `<file>:<line> → missing §<slug>` locator, or `None` if the stderr isn't a stranding (some other
/// failure). duvet prints (across ANSI-boxed lines): `missing section "<slug>" in spec/<path>` and a
/// `[<file>:<line>:<col>]` source span. We stitch the slug + the source span; both may be soft-wrapped
/// across lines, so scan the whole text rather than a single line.
fn parse_stranded(stderr: &str) -> Option<String> {
    // Strip the ANSI box-drawing / prefix noise so wrapped fragments join cleanly.
    let flat: String = stderr
        .lines()
        .map(|l| l.trim_start_matches(|c: char| "│╭╮╰─·×✕ ".contains(c)))
        .collect::<Vec<_>>()
        .join(" ");
    if !flat.contains("missing section") {
        return None;
    }
    // slug: the text inside the first `"..."` after `missing section`.
    let slug = flat
        .split_once("missing section")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(s, _)| s.trim().to_string());
    // source span: `[<...>:<line>:<col>]` — take the first bracketed path:line.
    let span = flat
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(s, _)| s.trim().to_string());
    match (slug, span) {
        (Some(slug), Some(span)) => Some(format!("{span} → missing §{slug}")),
        (Some(slug), None) => Some(format!("missing §{slug}")),
        _ => Some("a citation points at a missing/renamed spec section".to_string()),
    }
}

/// The last `n` non-blank lines of a string, rejoined — for surfacing a report failure's tail with
/// signal, not padding. Whitespace-only lines are dropped BEFORE taking the last `n` (duvet's error
/// output is full of blank spacer lines, so a verbatim tail could be mostly empty).
fn last_lines(s: &str, n: usize) -> String {
    let non_blank: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = non_blank.len().saturating_sub(n);
    non_blank[start..].join("\n")
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

    #[test]
    fn parse_stranded_extracts_slug_and_span() {
        // A real duvet stranding error (ANSI box-drawn, soft-wrapped), like `prelude.rs:320`.
        let stderr = r#"
  ×   × missing section "an-open-sums-payload-may-be-schema-typed" in spec/
  │   │ capabilities/type-system.md
  │      ╭─[implementation/seed/crates/rcdzc/src/prelude.rs:320:9]
  │  320 │     //= spec/capabilities/type-system.md#an-open-sums-payload-may-
  │      ·                                             ╰── referenced here
  ╰─▶ encountered 1 errors
"#;
        let loc = parse_stranded(stderr).expect("should detect a stranding");
        assert!(
            loc.contains("an-open-sums-payload-may-be-schema-typed"),
            "slug: {loc}"
        );
        assert!(loc.contains("prelude.rs:320"), "span: {loc}");
    }

    #[test]
    fn parse_stranded_none_on_other_errors() {
        // A non-stranding failure must NOT be classified as a stranding (so it still hard-fails).
        assert!(parse_stranded("thread 'main' panicked at 'boom'").is_none());
        assert!(parse_stranded("").is_none());
    }

    #[test]
    fn last_lines_drops_blanks_and_takes_the_tail() {
        // Blank/whitespace-only lines are filtered BEFORE taking the last n (matches the doc).
        let s = "a\n\nb\n   \nc\n\n";
        assert_eq!(last_lines(s, 2), "b\nc"); // last 2 NON-blank, not "\n" padding
        assert_eq!(last_lines(s, 10), "a\nb\nc"); // fewer than n → all non-blank
        assert_eq!(last_lines("\n  \n\t\n", 3), ""); // all blank → empty
    }
}
