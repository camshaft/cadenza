//! `xtask-bench` — the runtime allocation benchmark, tracked against a committed baseline.
//!
//! Carved out of `xtask/src/bench.rs` (v-xtask-decompose, decompose-the-monolith). The measurements
//! live in ONE place — the `hot_op_allocation_ceilings` test in `cdz-runtime`, which drives each hot op
//! under a process-wide counting allocator and prints `ALLOC <name>: <count>`. This bin runs exactly
//! that test, parses its `ALLOC` lines, and diffs the counts against `spec/bench/.alloc-baseline`. So
//! the assertion guard (ceilings) and the tracked benchmark share one source of truth — no second copy
//! of the workload to drift.
//!
//! `--save` rewrites the baseline; otherwise a REGRESSION (any op over baseline + tolerance) exits
//! non-zero (same shape as `gate --check`); an op that IMPROVED is reported but never fails.
//!
//! Repo root from `CDZ_REPO_ROOT` (else cwd). No fleet lease (nix manages concurrency under benchCheck).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let save = std::env::args().skip(1).any(|a| a == "--save");

    let measured = measure(&repo);
    if measured.is_empty() {
        eprintln!("xtask bench: no ALLOC lines captured — did the benchmark test change?");
        std::process::exit(1);
    }

    let baseline_path = baseline_path(&repo);
    if save {
        std::fs::create_dir_all(baseline_path.parent().expect("baseline has a parent"))
            .expect("create spec/bench dir");
        std::fs::write(&baseline_path, serialize_baseline(&measured))
            .expect("write alloc baseline");
        println!(
            "xtask bench: wrote baseline ({} ops) → {}",
            measured.len(),
            baseline_path.display()
        );
        return;
    }

    let baseline_text = std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
        eprintln!(
            "xtask bench: no baseline at {} — run `xtask-bench --save` first.",
            baseline_path.display()
        );
        std::process::exit(1);
    });
    let baseline = parse_baseline(&baseline_text);

    let report = diff(&measured, &baseline);
    print_report(&measured, &baseline, &report);

    if report.regressed.is_empty() {
        println!(
            "bench: no regressions vs baseline ({} ops tracked)",
            measured.len()
        );
    } else {
        eprintln!(
            "bench: {} REGRESSION(S) vs baseline:",
            report.regressed.len()
        );
        for (name, base, current) in &report.regressed {
            eprintln!("  {name}: {base} → {current} (+{})", current - base);
        }
        eprintln!("If the increase is intended, re-record with `xtask-bench --save`.");
        std::process::exit(1);
    }
}

/// The committed baseline: `<repo>/spec/bench/.alloc-baseline`, one `count\tname` line per op.
fn baseline_path(repo: &Path) -> PathBuf {
    repo.join("spec/bench/.alloc-baseline")
}

/// Allowed slack above baseline before an op counts as a regression: 2% of the baseline (min 2), so
/// ordinary structural noise never trips the gate but a real per-op increase does. PURE.
fn tolerance(base: u64) -> i64 {
    ((base as f64 * 0.02).ceil() as i64).max(2)
}

/// Parse the `ALLOC <name>: <count>` lines out of the benchmark test's output. PURE. The FIRST line is
/// emitted with no newline after libtest's `test tests::… ... ` start banner, so the marker is searched
/// for ANYWHERE in the line (not just at the start) — else the first op is dropped.
fn parse_alloc_lines(output: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in output.lines() {
        if let Some(pos) = line.find("ALLOC ") {
            let rest = &line[pos + "ALLOC ".len()..];
            if let Some((name, count)) = rest.rsplit_once(':')
                && let Ok(n) = count.trim().parse::<u64>()
            {
                out.insert(name.trim().to_string(), n);
            }
        }
    }
    out
}

/// Serialize measured counts to the committed baseline text (`count\tname` per line + a header). PURE.
fn serialize_baseline(measured: &BTreeMap<String, u64>) -> String {
    let mut body = String::from(
        "# runtime allocation baseline — gross heap allocs per op-batch (count\\tname).\n\
         # Source: the `hot_op_allocation_ceilings` test in cdz-runtime. Regenerate with\n\
         # `xtask-bench --save`; check with `xtask-bench`. Lower is better.\n",
    );
    for (name, count) in measured {
        body.push_str(&format!("{count}\t{name}\n"));
    }
    body
}

/// Parse the committed baseline text back to counts (`count\tname` per line; `#` comments + blanks
/// skipped). PURE — the inverse of `serialize_baseline`.
fn parse_baseline(text: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some((count, name)) = line.split_once('\t')
            && let Ok(n) = count.trim().parse::<u64>()
        {
            out.insert(name.trim().to_string(), n);
        }
    }
    out
}

/// The outcome of comparing `measured` against `baseline`. PURE data (no I/O).
#[derive(Default)]
struct Diff {
    /// (name, baseline, current) for ops that rose above baseline + tolerance.
    regressed: Vec<(String, u64, u64)>,
    /// (name, baseline, current) for ops that fell below baseline.
    improved: Vec<(String, u64, u64)>,
    /// names present in the measurement but not the baseline.
    new: Vec<String>,
    /// names present in the baseline but gone from the measurement.
    dropped: Vec<String>,
}

/// Classify each measured op vs the baseline: a REGRESSION is `current > baseline + tolerance`; an
/// IMPROVEMENT is `current < baseline`; else unchanged. New / dropped names are reported too. PURE.
fn diff(measured: &BTreeMap<String, u64>, baseline: &BTreeMap<String, u64>) -> Diff {
    let mut d = Diff::default();
    for (name, &current) in measured {
        match baseline.get(name) {
            Some(&base) => {
                let delta = current as i64 - base as i64;
                if delta > tolerance(base) {
                    d.regressed.push((name.clone(), base, current));
                } else if delta < 0 {
                    d.improved.push((name.clone(), base, current));
                }
            }
            None => d.new.push(name.clone()),
        }
    }
    for name in baseline.keys() {
        if !measured.contains_key(name) {
            d.dropped.push(name.clone());
        }
    }
    d
}

/// Print the human-readable benchmark table + improvement note (regressions are surfaced by `main`).
fn print_report(measured: &BTreeMap<String, u64>, baseline: &BTreeMap<String, u64>, report: &Diff) {
    println!("\nruntime allocation benchmark (allocs per op-batch; lower is better)\n");
    println!(
        "  {:<28} {:>10} {:>10} {:>10}",
        "op", "current", "baseline", "delta"
    );
    println!("  {}", "─".repeat(60));
    for (name, &current) in measured {
        match baseline.get(name) {
            Some(&base) => {
                let delta = current as i64 - base as i64;
                let mark = if delta > tolerance(base) {
                    " ⬆ REGRESSION"
                } else if delta < 0 {
                    " ⬇ improved"
                } else {
                    ""
                };
                println!("  {name:<28} {current:>10} {base:>10} {delta:>+10}{mark}");
            }
            None => println!(
                "  {name:<28} {current:>10} {:>10} {:>10}  (new — not in baseline)",
                "-", "-"
            ),
        }
    }
    for name in &report.dropped {
        println!(
            "  {name:<28} {:>10} {:>10}   (dropped from the benchmark)",
            "-", baseline[name]
        );
    }
    println!();
    if !report.improved.is_empty() {
        println!(
            "  {} op(s) improved — run `xtask-bench --save` to record the new floor.",
            report.improved.len()
        );
    }
}

/// Run the `hot_op_allocation_ceilings` benchmark test in cdz-runtime (single-threaded, --ignored,
/// --nocapture) and parse its `ALLOC` lines. Exits non-zero if the test fails to build/run. IMPURE
/// (spawns cargo); the parsing is the pure `parse_alloc_lines`.
fn measure(repo: &Path) -> BTreeMap<String, u64> {
    // cdz-runtime is workspace-excluded, so run from its crate dir. RUST_MIN_STACK matches the suite's
    // deep-recursion needs. The counting allocator is process-wide → --test-threads=1, and the test is
    // #[ignore]d → --ignored. No fleet lease: nix manages concurrency (benchCheck), bare runs are rare.
    let rt = repo.join("implementation/seed/crates/cdz-runtime");
    let output = std::process::Command::new("cargo")
        .current_dir(&rt)
        .args([
            "test",
            "--release",
            "tests::hot_op_allocation_ceilings",
            "--",
            "--ignored",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("RUST_MIN_STACK", "67108864")
        .output()
        .unwrap_or_else(|e| {
            eprintln!("xtask bench: failed to spawn the allocation benchmark test: {e}");
            std::process::exit(1);
        });
    if !output.status.success() {
        eprintln!(
            "xtask bench: the allocation benchmark test failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::process::exit(1);
    }
    parse_alloc_lines(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerance_is_two_percent_min_two() {
        assert_eq!(tolerance(0), 2, "min slack is 2 even for a zero baseline");
        assert_eq!(tolerance(50), 2, "2% of 50 = 1 → floored to the min 2");
        assert_eq!(tolerance(100), 2, "2% of 100 = 2");
        assert_eq!(tolerance(1000), 20, "2% of 1000 = 20");
        assert_eq!(tolerance(3589), 72, "2% of 3589 = 71.78 → ceil 72");
    }

    #[test]
    fn parse_alloc_lines_handles_the_no_newline_first_line() {
        // The first ALLOC is glued to libtest's start banner with no newline — the marker must be found
        // mid-line, not only at line start, or the first op is dropped.
        let out = "\
running 1 test
test tests::hot_op_allocation_ceilings ... ALLOC map_insert x1000: 3589
ALLOC list_push x1000: 2000
ALLOC noise: not_a_number
test result: ok.";
        let m = parse_alloc_lines(out);
        assert_eq!(
            m.get("map_insert x1000"),
            Some(&3589),
            "mid-line first op parsed"
        );
        assert_eq!(m.get("list_push x1000"), Some(&2000));
        assert!(
            !m.contains_key("noise"),
            "a non-numeric count is skipped, not panicked"
        );
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn baseline_round_trips_through_serialize_and_parse() {
        let measured: BTreeMap<String, u64> = [
            ("map_insert".to_string(), 3589u64),
            ("list_push".to_string(), 2000),
        ]
        .into_iter()
        .collect();
        let text = serialize_baseline(&measured);
        // Header comments present; each op as `count\tname`.
        assert!(text.starts_with("# runtime allocation baseline"));
        assert!(text.contains("3589\tmap_insert\n"));
        // Round-trip: parse it back → identical map (comments/blanks ignored).
        assert_eq!(parse_baseline(&text), measured);
        // Blank lines + comments are tolerated.
        assert_eq!(parse_baseline("# c\n\n3589\tmap_insert\n"), {
            let mut m = BTreeMap::new();
            m.insert("map_insert".to_string(), 3589);
            m
        });
    }

    #[test]
    fn diff_classifies_regression_improvement_new_and_dropped() {
        let baseline: BTreeMap<String, u64> = [
            ("steady".to_string(), 100u64),
            ("regressed".to_string(), 100),
            ("improved".to_string(), 100),
            ("dropped".to_string(), 100),
        ]
        .into_iter()
        .collect();
        let measured: BTreeMap<String, u64> = [
            ("steady".to_string(), 101u64), // +1, within tolerance(100)=2 → not a regression
            ("regressed".to_string(), 103), // +3 > tolerance 2 → REGRESSION
            ("improved".to_string(), 90),   // < baseline → improved
            ("new_op".to_string(), 5),      // not in baseline → new
        ]
        .into_iter()
        .collect();
        let d = diff(&measured, &baseline);
        assert_eq!(d.regressed, vec![("regressed".to_string(), 100, 103)]);
        assert_eq!(d.improved, vec![("improved".to_string(), 100, 90)]);
        assert_eq!(d.new, vec!["new_op".to_string()]);
        assert_eq!(d.dropped, vec!["dropped".to_string()]);
        // `steady` (+1, within tolerance) is in none of the buckets.
    }
}
