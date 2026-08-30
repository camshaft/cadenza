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

    (Verdict::Pass, String::new())
}

/// Run `gate-syntax`. Returns the process exit code.
pub fn gate_syntax(paths: &Paths, opts: &GateSyntaxOpts) -> i32 {
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
        let path = baseline_path(&paths.repo);
        if let Err(e) = std::fs::write(&path, &text) {
            eprintln!("gate-syntax --save: writing {}: {e}", path.display());
            return 2;
        }
        println!(
            "gate-syntax --save: wrote {} ({} cases)",
            path.display(),
            verdicts.len()
        );
        return 0;
    }

    if opts.check {
        return check_baseline(&paths.repo, &verdicts, subset);
    }

    // No `--check`/`--save`: fail on an outright Fail (the miscompile guard), else succeed.
    if n_fail > 0 {
        eprintln!("gate-syntax: {n_fail} case(s) FAILED");
        1
    } else {
        0
    }
}

/// Compare `verdicts` against the committed baseline. Mirrors the semantics gate's `check_baseline`:
/// reds on a `Pass → not-Pass` regression, a `Fail` whose baseline is not `pass`/`fail` (the gate-hole
/// guard), and — on a FULL run only — a vanished baseline case. `Todo → Pass` and new cases are additive.
fn check_baseline(repo: &Path, verdicts: &[(String, Verdict)], subset: bool) -> i32 {
    let path = baseline_path(repo);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "gate-syntax --check: no baseline at {} (create it with `gate-syntax --save`)",
                path.display()
            );
            return 2;
        }
    };
    let mut base: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut conflicting: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
            && let Some(prev) = base.insert(d.to_string(), verdict)
            && prev != verdict
        {
            conflicting.push(d.to_string());
        }
    }
    if !conflicting.is_empty() {
        conflicting.sort();
        conflicting.dedup();
        eprintln!(
            "gate-syntax --check: {} CONFLICTING duplicate title(s) in {} (same title, different \
             verdicts — regenerate with `gate-syntax --save`):",
            conflicting.len(),
            path.display()
        );
        for d in &conflicting {
            eprintln!("  •  {d}");
        }
        return 3;
    }

    let now: BTreeMap<&str, Verdict> = verdicts.iter().map(|(d, v)| (d.as_str(), *v)).collect();
    let mut regressed = Vec::new();
    let mut vanished = Vec::new();
    for (desc, &was) in &base {
        match now.get(desc.as_str()) {
            None => {
                if !subset {
                    vanished.push(desc.clone());
                }
            }
            Some(&is) if was == Verdict::Pass && is != Verdict::Pass => {
                regressed.push(format!("{desc} ({} → {})", was.tag(), is.tag()));
            }
            Some(_) => {}
        }
    }
    // A `Fail` whose baseline is neither `pass` (caught above) nor `fail` (a tracked known-fail) reds —
    // the gate-hole guard: a `todo`/absent case that now fails must not slip past the pass-regression rule.
    let failing: Vec<&str> = verdicts
        .iter()
        .filter(|(d, v)| {
            *v == Verdict::Fail
                && !matches!(base.get(d.as_str()), Some(Verdict::Pass | Verdict::Fail))
        })
        .map(|(d, _)| d.as_str())
        .collect();

    if !regressed.is_empty() {
        println!("REGRESSED ({}):", regressed.len());
        for r in &regressed {
            println!("  -  {r}");
        }
    }
    if !vanished.is_empty() {
        println!("vanished from the corpus ({}):", vanished.len());
        for v in &vanished {
            println!("  ?  {v}");
        }
    }
    if !failing.is_empty() {
        println!("FAILING ({}):", failing.len());
        for f in &failing {
            println!("  x  {f}");
        }
    }

    if regressed.is_empty() && vanished.is_empty() && failing.is_empty() {
        println!("gate-syntax --check: OK (no regressions vs baseline)");
        0
    } else {
        println!(
            "gate-syntax --check: FAIL ({} regressed, {} vanished, {} failing)",
            regressed.len(),
            vanished.len(),
            failing.len()
        );
        1
    }
}
