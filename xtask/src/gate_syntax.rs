//! `cargo xtask gate-syntax` — the parser/printer golden corpus grader (DESIGN-parser-test-corpus.md §4).
//!
//! Enumerates `spec/syntax/<surface>/<case>/` directories and grades each against the reference `cdz`
//! tool — the SAME shell-the-built-binary pattern the semantics `gate()` uses (`xtask` deps no syntax
//! crate; it drives the real `cdz`). For each case:
//!
//!   `cdz convert --to sexpr --structural <input>`  vs  `tree.sexp`     (bytes)
//!   `cdz fmt --stdout <input>`                      vs  `format.<ext>`  (or `input` if absent) (bytes)
//!
//! Verdicts are the additive `Pass`/`Todo`/`Fail` ladder (shared `xtask_support::Verdict`):
//! - `Pass`  — both comparisons match.
//! - `Todo`  — the reader DECLINES the input (a clean parse error, exit non-zero): a not-yet-realized
//!   surface/feature, never a false fail. This is the delanguaging move for the future Cadenza-parser
//!   rewrite — a construct it hasn't reached declines → `Todo`, it does not miscompile.
//! - `Fail`  — a wrong tree/format (mismatch), a missing `tree.sexp`, or an ICE.
//!
//! Compared against `spec/syntax/.gate-baseline` exactly as the semantics gate's `check_baseline`:
//! only `Pass → not-Pass` regresses; `Todo → Pass` is a silent additive tighten; a FULL run also reds a
//! vanished baseline case (a silently dropped/renamed case), while a SUBSET run (`--files`/`--case`)
//! skips the vanished check (the case lives in another selection). Line format `<verdict>\t<title>`,
//! union-mergeable — identical to the semantics baseline so one diff/merge tooling serves both.
//!
//! DURABLE HYGIENE RULE (see `spec/syntax/README.md`): a PR that renames a case or flips a verdict MUST
//! co-update `.gate-baseline` in the SAME PR, else the vanished/regression check reds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use xtask_support::{Verdict, serialize_baseline};

use crate::Paths;

/// Options for `gate-syntax` (mirrors the semantics gate's selection + save/check flags).
pub struct GateSyntaxOpts {
    /// Limit to these case directories (relative to `spec/syntax/` or absolute); empty = the whole corpus.
    pub files: Vec<PathBuf>,
    /// Substring filter on the case title (e.g. `comment`); `None` = no filter.
    pub case: Option<String>,
    /// Rewrite `spec/syntax/.gate-baseline` from the current verdicts (canonical sorted form).
    pub save: bool,
    /// Compare against the committed baseline and exit non-zero on a regression/vanished/failing case.
    pub check: bool,
    /// Read PRE-HARVESTED `<verdict>\t<title>` verdicts from this file and fold them against the
    /// committed baseline — WITHOUT re-grading via `cdz`. This is the entry the per-case nix aggregate
    /// (`.#checks.<arch>-linux.syntax-corpus`, inc-3c) uses: each cached per-case derivation emits its
    /// verdict line, the aggregate concatenates them, and this folds the whole set through the SAME
    /// `check_baseline` compare the live `--check` uses (single-sourced fold — the nix path never gets a
    /// divergent/weaker verdict comparison). Mutually exclusive with grading (ignores `--files`/`--case`).
    pub compare: Option<PathBuf>,
    /// Override the baseline file path (default `spec/syntax/.gate-baseline` under the repo). The per-case
    /// nix aggregate needs this: `xtaskBin` runs OUTSIDE a repo/git tree, so the default repo-relative
    /// resolution can't find the committed baseline — the aggregate passes `--baseline
    /// ${./spec/syntax/.gate-baseline}` explicitly. Applies to `--check`/`--compare`/`--save`.
    pub baseline: Option<PathBuf>,
}

/// Parse `<verdict>\t<title>` lines (the baseline / harvested-verdict format) into `(title, verdict)`
/// pairs; `#`/blank lines and unparseable lines are skipped. Shared by the `--compare` aggregate entry.
fn parse_verdicts(text: &str) -> Vec<(String, Verdict)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            l.split_once('\t')
                .and_then(|(v, d)| Verdict::parse(v).map(|verdict| (d.to_string(), verdict)))
        })
        .collect()
}

/// The syntax corpus root under `repo`.
fn corpus_root(repo: &Path) -> PathBuf {
    repo.join("spec/syntax")
}

/// The baseline file path.
fn baseline_path(repo: &Path) -> PathBuf {
    corpus_root(repo).join(".gate-baseline")
}

/// Resolve the reference `cdz` binary: the prebuilt `CDZ_SEED_BIN_DIR` (a nix app supplies it) else a
/// `cargo build -p cdz` under `target/debug`. Same override the semantics `build_tools` honors.
fn resolve_cdz(repo: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("CDZ_SEED_BIN_DIR") {
        return Ok(PathBuf::from(dir).join("cdz"));
    }
    let status = Command::new("cargo")
        .args(["build", "--quiet", "-p", "cdz"])
        .current_dir(repo)
        .status()
        .map_err(|e| format!("building cdz: {e}"))?;
    if !status.success() {
        return Err("building cdz failed".into());
    }
    Ok(repo.join("target/debug/cdz"))
}

/// The single `input.*` file in a case directory, with its extension (surface implied).
fn find_input(case: &Path) -> Option<(PathBuf, String)> {
    for entry in std::fs::read_dir(case).ok()?.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_str()?;
        let ext = path.extension()?.to_str()?.to_string();
        if name == format!("input.{ext}") {
            return Some((path, ext));
        }
    }
    None
}

/// Enumerate `spec/syntax/<surface>/<NN-name>/` case directories, sorted, applying `--files`/`--case`.
fn enumerate_cases(root: &Path, opts: &GateSyntaxOpts) -> Vec<PathBuf> {
    // `--files` restricts to the given dirs (resolved under the root if relative); else the whole corpus.
    let mut cases: Vec<PathBuf> = if opts.files.is_empty() {
        let mut all = Vec::new();
        for surface in sorted_subdirs(root) {
            all.extend(sorted_subdirs(&surface));
        }
        all
    } else {
        opts.files
            .iter()
            .map(|f| {
                if f.is_absolute() {
                    f.clone()
                } else {
                    root.join(f)
                }
            })
            .collect()
    };
    if let Some(needle) = &opts.case {
        cases.retain(|c| c.to_string_lossy().contains(needle.as_str()));
    }
    cases
}

/// Immediate subdirectories of `dir`, sorted.
fn sorted_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// A case's title for the baseline — its path relative to `spec/syntax/` (`sexp/04-comment-leading`).
fn case_title(root: &Path, case: &Path) -> String {
    case.strip_prefix(root)
        .unwrap_or(case)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Grade one case by driving the reference `cdz`. Returns `(verdict, detail)`.
fn grade_case(cdz: &Path, case: &Path) -> (Verdict, String) {
    let Some((input, ext)) = find_input(case) else {
        return (Verdict::Fail, "no input.<ext> file".into());
    };

    // Structural parse tree: `cdz convert --to sexpr --structural <input>`. A non-zero exit is a parse
    // DECLINE → Todo (not a fail). Success → compare stdout to tree.sexp.
    let conv = match Command::new(cdz)
        .args(["convert", "--to", "sexpr", "--structural"])
        .arg(&input)
        .output()
    {
        Ok(o) => o,
        Err(e) => return (Verdict::Fail, format!("running cdz convert: {e}")),
    };
    if !conv.status.success() {
        return (
            Verdict::Todo,
            format!(
                "parser declines: {}",
                String::from_utf8_lossy(&conv.stderr).trim()
            ),
        );
    }
    let tree_path = case.join("tree.sexp");
    match std::fs::read(&tree_path) {
        Ok(golden) if golden == conv.stdout => {}
        Ok(_) => {
            return (
                Verdict::Fail,
                "tree.sexp mismatch — structural render differs from the golden".into(),
            );
        }
        Err(_) => return (Verdict::Fail, "tree.sexp missing (bless it)".into()),
    }

    // Canonical format: `cdz fmt --stdout <input>` (surface inferred from the extension) vs
    // format.<ext>-or-input.
    let fmt = match Command::new(cdz)
        .args(["fmt", "--stdout"])
        .arg(&input)
        .output()
    {
        Ok(o) => o,
        Err(e) => return (Verdict::Fail, format!("running cdz fmt: {e}")),
    };
    if !fmt.status.success() {
        return (
            Verdict::Fail,
            format!(
                "cdz fmt failed: {}",
                String::from_utf8_lossy(&fmt.stderr).trim()
            ),
        );
    }
    let format_path = case.join(format!("format.{ext}"));
    let want = if format_path.exists() {
        std::fs::read(&format_path).unwrap_or_default()
    } else {
        std::fs::read(&input).unwrap_or_default()
    };
    if fmt.stdout != want {
        let which = if format_path.exists() {
            format!("format.{ext}")
        } else {
            "input (asserted canonical)".into()
        };
        return (Verdict::Fail, format!("fmt mismatch vs {which}"));
    }

    // Codemod goldens (the operator-blessed codemod corpus): for each `normalize.<pass>.<ext>` file in
    // the case dir, run `cdz normalize --<pass> --stdout <input>` and compare byte-exact. Pins that a
    // surface CODEMOD (`cdz normalize` pass, e.g. `--match-to-let`) produces the recorded output — the
    // transform the Cadenza rewrite must reproduce, so the language is SPECIFIED not just implemented.
    // Same-surface (like fmt); the pass name is the filename segment between `normalize.` and `.<ext>`.
    for entry in std::fs::read_dir(case).into_iter().flatten().flatten() {
        let fname = entry.file_name();
        let Some(fname) = fname.to_str() else {
            continue;
        };
        let Some(rest) = fname.strip_prefix("normalize.") else {
            continue;
        };
        let Some(pass) = rest.strip_suffix(&format!(".{ext}")) else {
            continue;
        };
        if pass.is_empty() {
            continue;
        }
        let norm = match Command::new(cdz)
            .args(["normalize", &format!("--{pass}"), "--stdout"])
            .arg(&input)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                return (
                    Verdict::Fail,
                    format!("running cdz normalize --{pass}: {e}"),
                );
            }
        };
        if !norm.status.success() {
            return (
                Verdict::Fail,
                format!(
                    "cdz normalize --{pass} failed: {}",
                    String::from_utf8_lossy(&norm.stderr).trim()
                ),
            );
        }
        if norm.stdout != std::fs::read(entry.path()).unwrap_or_default() {
            return (
                Verdict::Fail,
                format!("normalize --{pass} mismatch vs {fname}"),
            );
        }
    }

    (Verdict::Pass, String::new())
}

/// Run `gate-syntax`. Returns the process exit code.
pub fn gate_syntax(paths: &Paths, opts: &GateSyntaxOpts) -> i32 {
    // The baseline file — an explicit `--baseline` override (the per-case nix aggregate passes it, since
    // `xtaskBin` runs outside a repo tree) else the repo-relative default.
    let baseline = opts
        .baseline
        .clone()
        .unwrap_or_else(|| baseline_path(&paths.repo));

    // `--compare <file>`: fold PRE-HARVESTED verdicts against the baseline, no `cdz` re-grading (the
    // per-case nix aggregate's entry). A full-corpus fold — subset=false so the vanished check applies.
    // Resolved BEFORE the corpus-root check: the aggregate runs it with no `spec/syntax/` tree present.
    if let Some(vpath) = &opts.compare {
        let text = match std::fs::read_to_string(vpath) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("gate-syntax --compare: reading {}: {e}", vpath.display());
                return 2;
            }
        };
        let verdicts = parse_verdicts(&text);
        if verdicts.is_empty() {
            eprintln!(
                "gate-syntax --compare: no `<verdict>\\t<title>` lines in {}",
                vpath.display()
            );
            return 2;
        }
        return check_baseline(&baseline, &verdicts, false);
    }

    let root = corpus_root(&paths.repo);
    if !root.is_dir() {
        eprintln!("gate-syntax: no corpus at {}", root.display());
        return 2;
    }

    let cdz = match resolve_cdz(&paths.repo) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gate-syntax: {e}");
            return 2;
        }
    };
    let subset = !opts.files.is_empty() || opts.case.is_some();

    let cases = enumerate_cases(&root, opts);
    if cases.is_empty() {
        eprintln!("gate-syntax: no cases selected under {}", root.display());
        return 2;
    }

    let mut verdicts: Vec<(String, Verdict)> = Vec::new();
    let (mut n_pass, mut n_todo, mut n_fail) = (0usize, 0usize, 0usize);
    for case in &cases {
        let title = case_title(&root, case);
        let (verdict, detail) = grade_case(&cdz, case);
        match verdict {
            Verdict::Pass => n_pass += 1,
            Verdict::Todo => n_todo += 1,
            Verdict::Fail => n_fail += 1,
        }
        let tag = verdict.tag();
        if detail.is_empty() {
            println!("{tag}\t{title}");
        } else {
            println!("{tag}\t{title}  — {detail}");
        }
        verdicts.push((title, verdict));
    }
    println!(
        "\ngate-syntax: {n_pass} pass, {n_todo} todo, {n_fail} fail ({} cases)",
        cases.len()
    );

    if opts.save {
        let by_desc: BTreeMap<String, Verdict> =
            verdicts.iter().map(|(d, v)| (d.clone(), *v)).collect();
        let text = serialize_baseline(&by_desc);
        if let Err(e) = std::fs::write(&baseline, &text) {
            eprintln!("gate-syntax --save: writing {}: {e}", baseline.display());
            return 2;
        }
        println!(
            "gate-syntax --save: wrote {} ({} cases)",
            baseline.display(),
            verdicts.len()
        );
        return 0;
    }

    if opts.check {
        return check_baseline(&baseline, &verdicts, subset);
    }

    // No `--check`/`--save`: fail on an outright Fail (the miscompile guard), else succeed.
    if n_fail > 0 {
        eprintln!("gate-syntax: {n_fail} case(s) FAILED");
        1
    } else {
        0
    }
}

/// The outcome of comparing the current verdicts against a baseline text — the PURE core of
/// `--check`, split out so its invariants are unit-testable without touching the filesystem. Mirrors
/// the semantics gate's `check_baseline` invariants (xtask/src/main.rs), which were hard-won:
///  1. CONFLICTING (same title, DIFFERENT verdicts) baseline dups → `conflict = true` (exit 3): a
///     `merge=union` file can carry a dup LINE, and a map-keyed load silently masks one verdict.
///     A BENIGN dup (same verdict both copies) is a routine merge artifact — deduped in memory
///     (`benign_dups` counts them) and NOT a failure.
///  3. The FAILING-hole guard: a current `Fail` whose baseline is NEITHER `pass` (a regression, caught
///     separately) NOR `fail` (a tracked known-fail) reds — a `todo`/absent case that now fails must
///     not slip past the pass-regression rule (v-nix 2026-08-27). Applies regardless of `subset`.
///  4. TRACKED KNOWN-FAIL: a `fail` verdict against an explicit `fail` baseline is a deliberate,
///     git-committed pin — reported (`tracked_fail`) for visibility but NOT a gate failure; a later
///     PASS shows up in `gained`, prompting a re-baseline.
///  5. VANISHED (a baseline title with no current case) reds only on a FULL run; a `subset` run
///     (`--files`/`--case`) skips it (the case lives in another selection).
#[derive(Debug, Default, PartialEq)]
struct BaselineCompare {
    /// `pass → not-pass` regressions, formatted `title (was → now)`.
    regressed: Vec<String>,
    /// Baseline titles absent from this run (full run only).
    vanished: Vec<String>,
    /// Current fails not covered by a `pass`/`fail` baseline (the gate-hole guard).
    failing: Vec<String>,
    /// Current fails pinned by an explicit `fail` baseline — visible, not gate-redding.
    tracked_fail: Vec<String>,
    /// Cases that went `not-pass → pass` — additive, reported but never failing.
    gained: Vec<String>,
    /// A CONFLICTING duplicate title in the baseline (different verdicts) — a hard integrity error.
    conflict: Vec<String>,
    /// Count of BENIGN same-verdict duplicate lines (a `merge=union` artifact) — harmless.
    benign_dups: usize,
}

impl BaselineCompare {
    /// The process exit code: 3 on a conflicting-dup integrity error, 1 on any regression/vanished/
    /// failing, else 0 (`tracked_fail`/`gained`/`benign_dups` never red).
    fn exit_code(&self) -> i32 {
        if !self.conflict.is_empty() {
            3
        } else if !self.regressed.is_empty()
            || !self.vanished.is_empty()
            || !self.failing.is_empty()
        {
            1
        } else {
            0
        }
    }
}

/// Compare current `verdicts` against `baseline_text` — the PURE, I/O-free core carrying the five
/// invariants documented on [`BaselineCompare`]. `subset` is a `--files`/`--case` run (skips vanished).
fn compare_baseline(
    verdicts: &[(String, Verdict)],
    baseline_text: &str,
    subset: bool,
) -> BaselineCompare {
    let mut out = BaselineCompare::default();
    // Parse the baseline into a title→verdict map, splitting BENIGN (same-verdict) from CONFLICTING
    // (different-verdict) duplicate lines as we go.
    let mut base: BTreeMap<String, Verdict> = BTreeMap::new();
    for line in baseline_text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            match base.insert(d.to_string(), verdict) {
                None => {}
                Some(prev) if prev == verdict => out.benign_dups += 1,
                Some(_) => out.conflict.push(d.to_string()),
            }
        }
    }
    if !out.conflict.is_empty() {
        out.conflict.sort();
        out.conflict.dedup();
        return out; // a conflicting baseline is an integrity error — do not compare against it.
    }

    let now: BTreeMap<&str, Verdict> = verdicts.iter().map(|(d, v)| (d.as_str(), *v)).collect();
    for (desc, &was) in &base {
        match now.get(desc.as_str()) {
            None => {
                if !subset {
                    out.vanished.push(desc.clone());
                }
            }
            Some(&is) if was == Verdict::Pass && is != Verdict::Pass => {
                out.regressed
                    .push(format!("{desc} ({} → {})", was.tag(), is.tag()));
            }
            Some(&is) if was != Verdict::Pass && is == Verdict::Pass => {
                out.gained.push(desc.clone())
            }
            Some(_) => {}
        }
    }
    for (d, v) in verdicts {
        if *v != Verdict::Fail {
            continue;
        }
        match base.get(d.as_str()) {
            // A `fail` baseline is a deliberate tracked known-fail — visible, not redding.
            Some(Verdict::Fail) => out.tracked_fail.push(d.clone()),
            // A `pass` baseline that now fails is already a regression (caught above).
            Some(Verdict::Pass) => {}
            // A `todo` baseline or an absent case that now fails reds — the gate-hole guard.
            _ => out.failing.push(d.clone()),
        }
    }
    out
}

/// Compare `verdicts` against the committed baseline file, print the report, and return the exit code.
/// The I/O + reporting shell around the pure [`compare_baseline`].
fn check_baseline(path: &Path, verdicts: &[(String, Verdict)], subset: bool) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "gate-syntax --check: no baseline at {} (create it with `gate-syntax --save`)",
                path.display()
            );
            return 2;
        }
    };
    let cmp = compare_baseline(verdicts, &text, subset);

    if !cmp.conflict.is_empty() {
        eprintln!(
            "gate-syntax --check: {} CONFLICTING duplicate title(s) in {} (same title, different \
             verdicts — the map-keyed load silently masks one; regenerate with `gate-syntax --save`):",
            cmp.conflict.len(),
            path.display()
        );
        for d in &cmp.conflict {
            eprintln!("  •  {d}");
        }
        return 3;
    }
    if cmp.benign_dups > 0 {
        // Benign same-verdict dups are a `merge=union` artifact — harmless (deduped in memory for the
        // compare). `--check` is READ-ONLY: rewriting here would leave a dirty worktree and block every
        // agent's `fleet sync`. Dedup-on-disk is `gate-syntax --save`'s job.
        eprintln!(
            "gate-syntax --check: {} benign (same-verdict) duplicate line(s) in {} — a merge=union \
             artifact, harmless (deduped in memory). Run `gate-syntax --save` to rewrite clean.",
            cmp.benign_dups,
            path.display()
        );
    }
    if !cmp.gained.is_empty() {
        println!("newly passing ({}):", cmp.gained.len());
        for g in &cmp.gained {
            println!("  +  {g}");
        }
    }
    if !cmp.regressed.is_empty() {
        println!("REGRESSED ({}):", cmp.regressed.len());
        for r in &cmp.regressed {
            println!("  -  {r}");
        }
    }
    if !cmp.vanished.is_empty() {
        println!("vanished from the corpus ({}):", cmp.vanished.len());
        for v in &cmp.vanished {
            println!("  ?  {v}");
        }
    }
    if !cmp.failing.is_empty() {
        println!("FAILING ({}):", cmp.failing.len());
        for f in &cmp.failing {
            println!("  x  {f}");
        }
    }
    if !cmp.tracked_fail.is_empty() {
        // Visible but NOT redding — git-committed known-wrong pins (a deferred-fix repro).
        println!(
            "KNOWN-FAIL — tracked known-wrong (baseline `fail`), not a gate failure ({}):",
            cmp.tracked_fail.len()
        );
        for f in &cmp.tracked_fail {
            println!("  ⊗  {f}");
        }
    }

    let code = cmp.exit_code();
    if code == 0 {
        println!(
            "gate-syntax --check: OK (no regressions vs baseline; {} newly passing)",
            cmp.gained.len()
        );
    } else {
        println!(
            "gate-syntax --check: FAIL ({} regressed, {} vanished, {} failing)",
            cmp.regressed.len(),
            cmp.vanished.len(),
            cmp.failing.len()
        );
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pairs: &[(&str, Verdict)]) -> Vec<(String, Verdict)> {
        pairs.iter().map(|(d, v)| (d.to_string(), *v)).collect()
    }

    #[test]
    fn pass_to_not_pass_is_the_only_regression() {
        // pass→fail and pass→todo REGRESS; todo→pass GAINS; a steady pass/todo is quiet.
        let baseline = "pass\ta\npass\tb\ntodo\tc\ntodo\td\n";
        let now = v(&[
            ("a", Verdict::Fail), // pass→fail: regressed
            ("b", Verdict::Todo), // pass→todo: regressed
            ("c", Verdict::Pass), // todo→pass: gained (additive)
            ("d", Verdict::Todo), // steady todo: quiet
        ]);
        let cmp = compare_baseline(&now, baseline, false);
        assert_eq!(
            cmp.regressed.len(),
            2,
            "pass→fail and pass→todo both regress"
        );
        assert_eq!(cmp.gained, vec!["c".to_string()], "todo→pass is a gain");
        // `b` is a pass→todo regression, NOT a `failing` (it is not a Fail verdict).
        assert!(cmp.failing.is_empty());
        assert_eq!(cmp.exit_code(), 1);
    }

    #[test]
    fn failing_hole_guard_reds_a_todo_or_absent_case_that_now_fails() {
        // The v-nix gate-hole: a todo-baselined or ABSENT case that now FAILs must red even though it is
        // not a pass→not-pass regression.
        let baseline = "todo\ta\n"; // `b` is absent from the baseline (a fresh case)
        let now = v(&[("a", Verdict::Fail), ("b", Verdict::Fail)]);
        let cmp = compare_baseline(&now, baseline, false);
        assert!(cmp.regressed.is_empty(), "neither was a baseline pass");
        assert_eq!(
            cmp.failing.len(),
            2,
            "a todo→fail AND an absent→fail both red (the gate-hole guard)"
        );
        assert_eq!(cmp.exit_code(), 1);
    }

    #[test]
    fn tracked_known_fail_is_visible_but_not_redding() {
        // A `fail` verdict against an explicit `fail` baseline is a deliberate pin — reported, not red.
        let baseline = "fail\ta\npass\tb\n";
        let now = v(&[("a", Verdict::Fail), ("b", Verdict::Pass)]);
        let cmp = compare_baseline(&now, baseline, false);
        assert_eq!(cmp.tracked_fail, vec!["a".to_string()]);
        assert!(
            cmp.failing.is_empty(),
            "a fail+fail-baseline is NOT the gate-hole failing set"
        );
        assert!(cmp.regressed.is_empty());
        assert_eq!(
            cmp.exit_code(),
            0,
            "a tracked known-fail does not red the gate"
        );
    }

    #[test]
    fn vanished_reds_only_on_a_full_run() {
        // A baseline title with no current case is vanished on a FULL run, ignored on a SUBSET run.
        let baseline = "pass\ta\npass\tb\n";
        let now = v(&[("a", Verdict::Pass)]); // `b` not run
        assert_eq!(
            compare_baseline(&now, baseline, false).vanished,
            vec!["b".to_string()],
            "full run flags the vanished case"
        );
        assert_eq!(compare_baseline(&now, baseline, false).exit_code(), 1);
        assert!(
            compare_baseline(&now, baseline, true).vanished.is_empty(),
            "subset run skips the vanished check"
        );
        assert_eq!(compare_baseline(&now, baseline, true).exit_code(), 0);
    }

    #[test]
    fn benign_dup_is_harmless_but_conflicting_dup_is_a_hard_error() {
        let now = v(&[("a", Verdict::Pass)]);
        // Benign: the same title+verdict twice (a merge=union artifact) — counted, not fatal.
        let benign = "pass\ta\npass\ta\n";
        let cmp = compare_baseline(&now, benign, false);
        assert_eq!(cmp.benign_dups, 1);
        assert!(cmp.conflict.is_empty());
        assert_eq!(cmp.exit_code(), 0, "a benign dup does not red");
        // Conflicting: the same title with DIFFERENT verdicts — a hard integrity error (exit 3).
        let conflicting = "pass\ta\ntodo\ta\n";
        let cmp = compare_baseline(&now, conflicting, false);
        assert_eq!(cmp.conflict, vec!["a".to_string()]);
        assert_eq!(cmp.exit_code(), 3, "a conflicting dup is exit 3");
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let baseline = "# header\n\npass\ta\n";
        let cmp = compare_baseline(&v(&[("a", Verdict::Pass)]), baseline, false);
        assert_eq!(cmp.exit_code(), 0);
        assert_eq!(cmp.benign_dups, 0);
    }

    #[test]
    fn parse_verdicts_reads_the_harvested_format_and_skips_noise() {
        // The `--compare` aggregate entry parses `<verdict>\t<title>` lines (the concatenated per-case
        // nix verdicts), skipping `#`/blank/garbage lines — the same vocabulary as the baseline file.
        let text = "# header\n\npass\tsexp/01\ttrailing-ignored?\ntodo\tsexp/17\nfail\tml/03\ngarbage line\nnope\tx\n";
        // `split_once('\t')` keeps everything after the FIRST tab as the title (so a stray tab stays in
        // the title, matching the baseline's own map-load); the garbage line (no tab) and the unknown
        // verdict tag `nope` are dropped. (Verdict isn't Debug, so compare via its tag.)
        let got: Vec<(String, &str)> = parse_verdicts(text)
            .into_iter()
            .map(|(d, v)| (d, v.tag()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("sexp/01\ttrailing-ignored?".to_string(), "pass"),
                ("sexp/17".to_string(), "todo"),
                ("ml/03".to_string(), "fail"),
            ]
        );
    }
}
