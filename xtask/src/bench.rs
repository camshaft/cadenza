//! `cargo xtask bench` — the runtime allocation benchmark, tracked over time against a committed
//! baseline (mirrors `gate --save`/`--check`).
//!
//! # Why allocation COUNT, not wall-clock
//! The value-heap runtime's shipped form is a wasm component behind the talc allocator; native
//! wall-clock timing would measure the system allocator and the host, not the shipped path. Allocation
//! COUNT, by contrast, is a property of the `Handle`-typed core itself — every `Box`/`Vec` call is the
//! same on native and wasm — so it is deterministic, reproducible across machines, and the exact lever
//! the runtime-optimization work pulls. This harness therefore tracks gross heap allocations per hot
//! operation. (A wall-clock Criterion pass could be layered on later for constant-factor work; the
//! allocation budget is the primary signal.)
//!
//! # How it runs
//! The measurements live in ONE place — the `hot_op_allocation_ceilings` test in `cdz-runtime`, which
//! drives each hot op under a process-wide counting allocator and prints `ALLOC <name>: <count>`. That
//! test is `#[ignore]`d (the counter is process-wide, so it must run single-threaded and alone). This
//! command runs exactly that test, parses its `ALLOC` lines, and diffs the counts against the committed
//! baseline `spec/bench/.alloc-baseline`. So the assertion guard (ceilings) and the tracked benchmark
//! share a single source of truth — no second copy of the workload to drift.

use crate::Paths;
use std::collections::BTreeMap;
use std::path::PathBuf;
use xshell::{Shell, cmd};

/// The committed baseline: `<repo>/spec/bench/.alloc-baseline`, one `count\tname` line per op.
fn baseline_path(paths: &Paths) -> PathBuf {
    paths.repo.join("spec/bench/.alloc-baseline")
}

/// Run the allocation benchmark. `save` rewrites the baseline; otherwise the counts are diffed against
/// it and a REGRESSION (any op allocating more than baseline + tolerance) exits non-zero — the same
/// shape as `gate --check`. An op that IMPROVED (allocates fewer) is reported but never fails.
pub(crate) fn run(paths: &Paths, save: bool) {
    let measured = measure(paths);
    if measured.is_empty() {
        eprintln!("xtask bench: no ALLOC lines captured — did the benchmark test change?");
        std::process::exit(1);
    }

    if save {
        save_baseline(paths, &measured);
        println!("xtask bench: wrote baseline ({} ops) → {}", measured.len(), baseline_path(paths).display());
        return;
    }

    let baseline = load_baseline(paths);
    println!("\nruntime allocation benchmark (allocs per op-batch; lower is better)\n");
    println!("  {:<28} {:>10} {:>10} {:>10}", "op", "current", "baseline", "delta");
    println!("  {}", "─".repeat(60));

    let mut regressed = Vec::new();
    let mut improved = Vec::new();
    for (name, &current) in &measured {
        match baseline.get(name) {
            Some(&base) => {
                let delta = current as i64 - base as i64;
                let mark = if delta > tolerance(base) {
                    regressed.push((name.clone(), base, current));
                    " ⬆ REGRESSION"
                } else if delta < 0 {
                    improved.push((name.clone(), base, current));
                    " ⬇ improved"
                } else {
                    ""
                };
                println!("  {name:<28} {current:>10} {base:>10} {delta:>+10}{mark}");
            }
            None => println!("  {name:<28} {current:>10} {:>10} {:>10}  (new — not in baseline)", "-", "-"),
        }
    }
    // A baseline op that vanished from the measurement is worth flagging (renamed/removed workload).
    for name in baseline.keys() {
        if !measured.contains_key(name) {
            println!("  {name:<28} {:>10} {:>10}   (dropped from the benchmark)", "-", baseline[name]);
        }
    }

    println!();
    if !improved.is_empty() {
        println!("  {} op(s) improved — run `cargo xtask bench --save` to record the new floor.", improved.len());
    }
    if regressed.is_empty() {
        println!("bench: no regressions vs baseline ({} ops tracked)", measured.len());
    } else {
        eprintln!("bench: {} REGRESSION(S) vs baseline:", regressed.len());
        for (name, base, current) in &regressed {
            eprintln!("  {name}: {base} → {current} (+{})", current - base);
        }
        eprintln!("If the increase is intended, re-record with `cargo xtask bench --save`.");
        std::process::exit(1);
    }
}

/// Allowed slack above baseline before an op counts as a regression: 2% of the baseline (min 2), so
/// ordinary structural noise never trips the gate but a real per-op increase does.
fn tolerance(base: u64) -> i64 {
    ((base as f64 * 0.02).ceil() as i64).max(2)
}

/// Run the `hot_op_allocation_ceilings` benchmark test in cdz-runtime (single-threaded, --ignored,
/// --nocapture) and parse its `ALLOC <name>: <count>` lines. Panics if the test fails to build/run.
fn measure(paths: &Paths) -> BTreeMap<String, u64> {
    let sh = Shell::new().expect("open a shell for the benchmark");
    let rt = paths.seed.join("crates/cdz-runtime");
    sh.change_dir(&rt);
    // cdz-runtime is workspace-excluded, so run from its crate dir. RUST_MIN_STACK matches the suite's
    // deep-recursion needs. The counting allocator is process-wide → --test-threads=1, and the test is
    // #[ignore]d → --ignored.
    let output = cmd!(
        sh,
        "cargo test --release tests::hot_op_allocation_ceilings -- --ignored --exact --nocapture --test-threads=1"
    )
    .env("RUST_MIN_STACK", "67108864")
    .read()
    .unwrap_or_else(|e| {
        eprintln!("xtask bench: failed to run the allocation benchmark test: {e}");
        std::process::exit(1);
    });

    let mut out = BTreeMap::new();
    for line in output.lines() {
        // A line looks like `ALLOC map_insert x1000: 3589`. The FIRST one is emitted with no newline
        // after libtest's `test tests::… ... ` start banner, so search for the `ALLOC ` marker
        // ANYWHERE in the line (not just at the start) — else the first op (map_insert) is dropped.
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

fn save_baseline(paths: &Paths, measured: &BTreeMap<String, u64>) {
    let path = baseline_path(paths);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create spec/bench dir");
    let mut body = String::from(
        "# runtime allocation baseline — gross heap allocs per op-batch (count\\tname).\n\
         # Source: the `hot_op_allocation_ceilings` test in cdz-runtime. Regenerate with\n\
         # `cargo xtask bench --save`; check with `cargo xtask bench`. Lower is better.\n",
    );
    for (name, count) in measured {
        body.push_str(&format!("{count}\t{name}\n"));
    }
    std::fs::write(&path, body).expect("write alloc baseline");
}

fn load_baseline(paths: &Paths) -> BTreeMap<String, u64> {
    let path = baseline_path(paths);
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("xtask bench: no baseline at {} — run `cargo xtask bench --save` first.", path.display());
        std::process::exit(1);
    };
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
