//! The CADENZA-BACKEND equivalence oracle dimension (operator seq-184).
//!
//! For each generated program `P`, compare two paths — both through the `cdz` binary + the same runtime
//! store, so the ONLY difference is the `--target cadenza` round-trip:
//!
//!   DIRECT:   `cdz compile P -o w.wasm`                                    → run → V1
//!   CADENZA:  `cdz compile P --target cadenza -o m.ast; cdz compile m.ast -o w.wasm` → run → V2
//!
//! `--target cadenza` re-emits the OPTIMIZED program (after resolution/inference/const-fold/optimization)
//! BACK to Cadenza binary AST; recompiling that to wasm MUST produce the same value. A divergence (V1≠V2,
//! or one traps and the other doesn't) is a real CADENZA-BACKEND miscompile — exactly the class the
//! wasm-vs-rust differential surfaces for the wasm/rust backends (S141, 22-0024), now for the cadenza
//! backend. This is the interim VALUE-EQ oracle; v-lean-oracle's functional-equivalence Lean oracle will
//! be an additional/stronger arm when it lands.
//!
//! NULLARY focus: `cdz run` needs `--arg` for a param'd `main`; a param'd program errors on BOTH paths
//! (→ [`Outcome::Skip`] on both → agree, skipped). So the comparison covers the (large) nullary space.

use std::path::Path;
use std::process::Command;

/// Per-`cdz`-invocation wall-clock cap (seconds). A generated program can HANG the compiler (deep
/// recursion) or the runtime; without a cap the blocking `Command::output()` would wedge the whole sweep
/// (observed at S179). Each compile/run is wrapped with `timeout -s KILL` so a hang is killed + the pair
/// is treated as non-comparable (Skip), and the sweep continues.
const CDZ_STEP_TIMEOUT_SECS: &str = "20";

/// A `cdz` command wrapped in `timeout -s KILL <CDZ_STEP_TIMEOUT_SECS>` so a hung step cannot wedge the
/// sweep. (`timeout` is coreutils, already relied on by the fuzz cycle.)
fn cdz_cmd(cdz: &Path) -> Command {
    let mut cmd = Command::new("timeout");
    cmd.arg("-s")
        .arg("KILL")
        .arg(CDZ_STEP_TIMEOUT_SECS)
        .arg(cdz);
    cmd
}

/// The outcome of running one compiled program via `cdz run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Ran to a rendered value (the trimmed stdout).
    Value(String),
    /// Trapped at runtime (non-zero exit + a trap on stderr).
    Trap,
    /// Not comparable for THIS program (a decline / compile-not-yet / param'd-main usage error / infra) —
    /// carries a reason. Never a mismatch: a skip on either side means the pair is skipped.
    Skip(String),
}

/// One dual-path comparison result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CzDiff {
    /// Both paths agree (same value, both trap, or either side skipped).
    Agree,
    /// The two paths DISAGREE — a cadenza-backend miscompile candidate. `direct`/`cadenza` render the sides.
    Mismatch { direct: String, cadenza: String },
}

/// Render an [`Outcome`] for a finding detail.
fn show(o: &Outcome) -> String {
    match o {
        Outcome::Value(v) => format!("value {v}"),
        Outcome::Trap => "trap".to_string(),
        Outcome::Skip(r) => format!("skip({r})"),
    }
}

/// Run `cdz compile <inputs…> [--target cadenza] -o <out>`; returns `Err(reason)` if the compile does not
/// produce an artifact (a front-end decline or a not-yet — a per-program SKIP condition, not infra).
fn cdz_compile(
    cdz: &Path,
    inputs: &[&Path],
    target_cadenza: bool,
    out: &Path,
) -> Result<(), String> {
    let mut cmd = cdz_cmd(cdz);
    cmd.arg("compile");
    for i in inputs {
        cmd.arg(i);
    }
    if target_cadenza {
        cmd.arg("--target").arg("cadenza");
    }
    cmd.arg("-o").arg(out);
    let output = cmd
        .output()
        .map_err(|e| format!("spawn `cdz compile` failed: {e}"))?;
    if !output.status.success() || !out.is_file() {
        return Err(first_line(&String::from_utf8_lossy(&output.stderr)));
    }
    Ok(())
}

/// Run a compiled wasm component via `cdz run <wasm>` (nullary) and classify the outcome.
fn cdz_run(cdz: &Path, wasm: &Path, store: &Path) -> Outcome {
    let output = match cdz_cmd(cdz)
        .arg("run")
        .arg(wasm)
        .env("CDZ_RUN_STORE", store)
        .output()
    {
        Ok(o) => o,
        Err(e) => return Outcome::Skip(format!("spawn `cdz run` failed: {e}")),
    };
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let v = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        return Outcome::Value(v);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("trap") || stderr.contains("unreachable") {
        Outcome::Trap
    } else {
        // A usage error (param'd main needs --arg), a missing-runtime, etc. — non-comparable for this run.
        Outcome::Skip(first_line(&stderr))
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// Compare the DIRECT-wasm outcome with the `--target cadenza` round-tripped outcome for one program.
/// `tmp` is a scratch directory (unique per call). Returns [`CzDiff::Agree`] when either side skips.
pub fn cadenza_diff(cdz: &Path, source: &str, store: &Path, tmp: &Path) -> CzDiff {
    let src_path = tmp.join("p.sexp");
    if std::fs::write(&src_path, source).is_err() {
        return CzDiff::Agree; // scratch write failed — skip (never a false mismatch)
    }

    // DIRECT path: source -> wasm.
    let direct_wasm = tmp.join("direct.wasm");
    let direct = match cdz_compile(cdz, &[&src_path], false, &direct_wasm) {
        Ok(()) => cdz_run(cdz, &direct_wasm, store),
        // The FRONT-END declined P — nothing to compare (both paths share the front-end). Skip.
        Err(r) => {
            return CzDiff::from_sides(
                &Outcome::Skip(format!("direct-compile: {r}")),
                &Outcome::Skip(String::new()),
            );
        }
    };
    // A direct skip (e.g. param'd main) → nothing to compare.
    if let Outcome::Skip(_) = direct {
        return CzDiff::Agree;
    }

    // CADENZA path: source -> .ast (--target cadenza) -> wasm.
    let ast_path = tmp.join("mid.ast");
    if let Err(r) = cdz_compile(cdz, &[&src_path], true, &ast_path) {
        // `--target cadenza` itself refused to emit — a cadenza-emit DECLINE (coverage gap, not a value
        // miscompile). Skip rather than false-flag.
        return CzDiff::from_sides(&direct, &Outcome::Skip(format!("cadenza-emit: {r}")));
    }
    let cadenza_wasm = tmp.join("cadenza.wasm");
    let cadenza = match cdz_compile(cdz, &[&ast_path], false, &cadenza_wasm) {
        Ok(()) => cdz_run(cdz, &cadenza_wasm, store),
        Err(r) => Outcome::Skip(format!("recompile-ast: {r}")),
    };
    CzDiff::from_sides(&direct, &cadenza)
}

impl CzDiff {
    /// Compare two outcomes: a `Skip` on EITHER side → `Agree` (non-comparable); both `Trap` → `Agree`;
    /// equal `Value` → `Agree`; anything else (differing values, or one traps + one values) → `Mismatch`.
    fn from_sides(direct: &Outcome, cadenza: &Outcome) -> CzDiff {
        match (direct, cadenza) {
            (Outcome::Skip(_), _) | (_, Outcome::Skip(_)) => CzDiff::Agree,
            (Outcome::Trap, Outcome::Trap) => CzDiff::Agree,
            (Outcome::Value(a), Outcome::Value(b)) if a == b => CzDiff::Agree,
            _ => CzDiff::Mismatch {
                direct: show(direct),
                cadenza: show(cadenza),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_sides_classification() {
        // Equal values agree.
        assert_eq!(
            CzDiff::from_sides(&Outcome::Value("7".into()), &Outcome::Value("7".into())),
            CzDiff::Agree
        );
        // Both trap agree.
        assert_eq!(
            CzDiff::from_sides(&Outcome::Trap, &Outcome::Trap),
            CzDiff::Agree
        );
        // A skip on either side is non-comparable → agree.
        assert_eq!(
            CzDiff::from_sides(&Outcome::Skip("x".into()), &Outcome::Value("7".into())),
            CzDiff::Agree
        );
        assert_eq!(
            CzDiff::from_sides(&Outcome::Value("7".into()), &Outcome::Skip("x".into())),
            CzDiff::Agree
        );
        // Differing values MISMATCH.
        assert!(matches!(
            CzDiff::from_sides(&Outcome::Value("7".into()), &Outcome::Value("8".into())),
            CzDiff::Mismatch { .. }
        ));
        // Value-vs-trap (liveness split) MISMATCH.
        assert!(matches!(
            CzDiff::from_sides(&Outcome::Value("7".into()), &Outcome::Trap),
            CzDiff::Mismatch { .. }
        ));
    }
}
