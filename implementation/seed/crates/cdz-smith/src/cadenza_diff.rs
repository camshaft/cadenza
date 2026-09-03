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

use crate::lean::EquivTrial;

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
    /// The step exceeded the per-call timeout ([`CDZ_STEP_TIMEOUT_SECS`]) — a HANG (operator seq-203: the
    /// compiler/runtime must never hang the sweep indefinitely). Captured as a hang-witness, not skipped.
    Timeout,
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
    /// A step HUNG (hit the per-call timeout) — a compiler/runtime non-termination witness. `at` names the
    /// step (direct-compile / cadenza-emit / recompile-ast / run). Persisted + routed to v-compiler-perf.
    Hang { at: &'static str },
}

/// True if the process was KILLED by `timeout -s KILL` (SIGKILL → exit 137 = 128+9) — i.e. it HUNG past
/// [`CDZ_STEP_TIMEOUT_SECS`].
fn is_timeout(status: &std::process::ExitStatus) -> bool {
    status.code() == Some(137)
}

/// Render an [`Outcome`] for a finding detail.
fn show(o: &Outcome) -> String {
    match o {
        Outcome::Value(v) => format!("value {v}"),
        Outcome::Trap => "trap".to_string(),
        Outcome::Timeout => "timeout".to_string(),
        Outcome::Skip(r) => format!("skip({r})"),
    }
}

/// A compile that did not produce an artifact: a HANG (timed out) or a per-program DECLINE (front-end
/// reject / not-yet — a SKIP condition, not infra).
enum CompileErr {
    Timeout,
    Decline(String),
}

/// Run `cdz compile <inputs…> [--target cadenza] -o <out>`.
fn cdz_compile(
    cdz: &Path,
    inputs: &[&Path],
    target_cadenza: bool,
    out: &Path,
) -> Result<(), CompileErr> {
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
        .map_err(|e| CompileErr::Decline(format!("spawn `cdz compile` failed: {e}")))?;
    if is_timeout(&output.status) {
        return Err(CompileErr::Timeout);
    }
    if !output.status.success() || !out.is_file() {
        return Err(CompileErr::Decline(first_line(&String::from_utf8_lossy(
            &output.stderr,
        ))));
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
    if is_timeout(&output.status) {
        return Outcome::Timeout;
    }
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

/// An early exit from the dual-path evaluation, before a side-by-side comparison is possible.
enum DualEarly {
    /// A compile/run step HUNG (hit the per-call timeout); `.0` names the step.
    Hang(&'static str),
    /// The pair is non-comparable (scratch-write failure, or the shared front-end declined the DIRECT
    /// path so there is nothing to round-trip).
    Uncomparable,
}

/// Run BOTH paths (direct-wasm and `--target cadenza` round-trip) and return each side's [`Outcome`],
/// or an early exit. Shared by [`cadenza_diff`] (value-eq classification) and [`cadenza_confirm`] (the
/// symbolic-equivalence confirm), so the two can never drift on how the paths are run.
fn run_dual_path(
    cdz: &Path,
    source: &str,
    store: &Path,
    tmp: &Path,
) -> Result<(Outcome, Outcome), DualEarly> {
    let src_path = tmp.join("p.sexp");
    if std::fs::write(&src_path, source).is_err() {
        return Err(DualEarly::Uncomparable); // scratch write failed — non-comparable
    }

    // DIRECT path: source -> wasm. A compile HANG here = the compiler hangs on P (seq-203 hang-witness).
    let direct_wasm = tmp.join("direct.wasm");
    let direct = match cdz_compile(cdz, &[&src_path], false, &direct_wasm) {
        Ok(()) => cdz_run(cdz, &direct_wasm, store),
        Err(CompileErr::Timeout) => return Err(DualEarly::Hang("direct-compile")),
        // The FRONT-END declined P — nothing to compare (both paths share the front-end).
        Err(CompileErr::Decline(_)) => return Err(DualEarly::Uncomparable),
    };
    // A run HANG on the direct side is also a hang-witness (runtime non-termination).
    if direct == Outcome::Timeout {
        return Err(DualEarly::Hang("direct-run"));
    }
    // A direct skip (e.g. param'd main) → nothing to compare.
    if let Outcome::Skip(_) = direct {
        return Err(DualEarly::Uncomparable);
    }

    // CADENZA path: source -> .ast (--target cadenza) -> wasm.
    let ast_path = tmp.join("mid.ast");
    match cdz_compile(cdz, &[&src_path], true, &ast_path) {
        Ok(()) => {}
        Err(CompileErr::Timeout) => return Err(DualEarly::Hang("cadenza-emit")),
        // `--target cadenza` refused to emit — a cadenza-emit DECLINE (coverage gap): the cadenza side
        // is a Skip, so the pair is non-comparable (matches the prior `from_sides(direct, Skip)=Agree`).
        Err(CompileErr::Decline(r)) => {
            return Ok((direct, Outcome::Skip(format!("cadenza-emit: {r}"))));
        }
    }
    let cadenza_wasm = tmp.join("cadenza.wasm");
    let cadenza = match cdz_compile(cdz, &[&ast_path], false, &cadenza_wasm) {
        Ok(()) => cdz_run(cdz, &cadenza_wasm, store),
        Err(CompileErr::Timeout) => return Err(DualEarly::Hang("recompile-ast")),
        Err(CompileErr::Decline(r)) => Outcome::Skip(format!("recompile-ast: {r}")),
    };
    if cadenza == Outcome::Timeout {
        return Err(DualEarly::Hang("cadenza-run"));
    }
    Ok((direct, cadenza))
}

/// Compare the DIRECT-wasm outcome with the `--target cadenza` round-tripped outcome for one program.
/// `tmp` is a scratch directory (unique per call). Returns [`CzDiff::Agree`] when either side skips.
pub fn cadenza_diff(cdz: &Path, source: &str, store: &Path, tmp: &Path) -> CzDiff {
    match run_dual_path(cdz, source, store, tmp) {
        Ok((direct, cadenza)) => CzDiff::from_sides(&direct, &cadenza),
        Err(DualEarly::Hang(at)) => CzDiff::Hang { at },
        Err(DualEarly::Uncomparable) => CzDiff::Agree,
    }
}

/// The `--target cadenza` round-trip AST for a program — the input to v-lean-oracle's `(equiv P P')`
/// symbolic-equivalence trial (the SAME `mid.ast` the value-eq [`cadenza_diff`] recompiles, exposed as
/// a decoded AST instead of run for a value). See [`equiv_trial_for`].
pub enum RoundtripAst {
    /// `--target cadenza` emitted the optimized program as binary AST, decoded here.
    Ast(cadenza_syntax::ast::Arenas),
    /// The front-end/cadenza-emit DECLINED (a coverage gap / `/cadenza-declined` marker) — not comparable.
    Declined(String),
    /// The `--target cadenza` compile HUNG (hit the per-call timeout) — a non-termination witness.
    Hang,
}

/// Produce the `--target cadenza` round-trip AST for `source`: run `cdz compile P --target cadenza -o
/// mid.ast` (the SAME step [`cadenza_diff`]'s cadenza path uses) and DECODE the emitted binary AST rather
/// than recompiling+running it. `tmp` is a unique scratch dir. A decline → [`RoundtripAst::Declined`], a
/// hang → [`RoundtripAst::Hang`], and an emitted-but-undecodable blob → `Declined` (never a panic).
pub fn cadenza_roundtrip_ast(cdz: &Path, source: &str, tmp: &Path) -> RoundtripAst {
    let src_path = tmp.join("p.sexp");
    if let Err(e) = std::fs::write(&src_path, source) {
        return RoundtripAst::Declined(format!("scratch write failed: {e}"));
    }
    let ast_path = tmp.join("mid.ast");
    match cdz_compile(cdz, &[&src_path], true, &ast_path) {
        Ok(()) => {}
        Err(CompileErr::Timeout) => return RoundtripAst::Hang,
        Err(CompileErr::Decline(r)) => return RoundtripAst::Declined(format!("cadenza-emit: {r}")),
    }
    let bytes = match std::fs::read(&ast_path) {
        Ok(b) => b,
        Err(e) => return RoundtripAst::Declined(format!("read mid.ast: {e}")),
    };
    match cadenza_syntax::codec::decode(&bytes) {
        Some(arenas) => RoundtripAst::Ast(arenas),
        None => RoundtripAst::Declined("mid.ast did not decode as binary AST".to_string()),
    }
}

/// Build an [`EquivTrial`] pairing `source`'s ORIGINAL AST with its `--target cadenza` round-trip — the
/// unit v-lean-oracle's symbolic-equivalence oracle proves (`(equiv orig cadenza)` HOLDS = the cadenza
/// backend preserved the program's meaning for ALL inputs). Returns `None` (SKIP this program) when the
/// source does not parse, or the round-trip DECLINED / HUNG (the `/cadenza-declined` case) — i.e. only a
/// cleanly round-tripped program yields a comparable equiv trial.
pub fn equiv_trial_for(cdz: &Path, source: &str, tmp: &Path) -> Option<EquivTrial> {
    let orig = cadenza_syntax::sexpr::read(source).ok()?;
    match cadenza_roundtrip_ast(cdz, source, tmp) {
        RoundtripAst::Ast(cadenza) => Some(EquivTrial { orig, cadenza }),
        RoundtripAst::Declined(_) | RoundtripAst::Hang => None,
    }
}

/// The refined result of the sampled confirm for an equiv sweep's suspected divergence — splits the
/// old "not-a-mismatch" outcome so the two very different reasons are distinguishable. See
/// [`cadenza_confirm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Both paths ran to comparable outcomes and DISAGREE — a CONFIRMED cadenza-backend divergence.
    Divergence { direct: String, cadenza: String },
    /// Both paths ran to the SAME value (or both trapped): the sampled values AGREE. For an equiv-sweep
    /// suspected divergence this is a SYMBOLIC FALSE-POSITIVE — the oracle's normal forms differ but the
    /// runtime values match, so it names a normalize rule the oracle is missing (v-lean-oracle's metric).
    ValuesAgree,
    /// A skip/decline on either side — the sampled net could NOT compare (e.g. a param'd main). Inherent
    /// non-comparability, not a symbolic false-positive and not a divergence.
    Uncomparable,
    /// A step hung during the confirm run.
    Hang,
}

/// Re-run the sampled value-eq paths and classify the result at higher resolution than [`cadenza_diff`]:
/// distinguish a real value AGREEMENT ([`ConfirmOutcome::ValuesAgree`]) from mere non-comparability
/// ([`ConfirmOutcome::Uncomparable`]). Used by the equiv sweep to CONFIRM a symbolic suspected divergence
/// and, when the sampled values agree, mark it a symbolic false-positive to hand back to v-lean-oracle.
pub fn cadenza_confirm(cdz: &Path, source: &str, store: &Path, tmp: &Path) -> ConfirmOutcome {
    match run_dual_path(cdz, source, store, tmp) {
        Ok((direct, cadenza)) => classify_confirm(&direct, &cadenza),
        Err(DualEarly::Hang(_)) => ConfirmOutcome::Hang,
        Err(DualEarly::Uncomparable) => ConfirmOutcome::Uncomparable,
    }
}

/// Classify two sampled [`Outcome`]s for the confirm (pure — unit-testable without a compiler). A `Skip`
/// on either side → `Uncomparable`; both trap or equal values → `ValuesAgree`; differing values or a
/// value-vs-trap liveness split → `Divergence`.
fn classify_confirm(direct: &Outcome, cadenza: &Outcome) -> ConfirmOutcome {
    match (direct, cadenza) {
        (Outcome::Skip(_) | Outcome::Timeout, _) | (_, Outcome::Skip(_) | Outcome::Timeout) => {
            ConfirmOutcome::Uncomparable
        }
        (Outcome::Trap, Outcome::Trap) => ConfirmOutcome::ValuesAgree,
        (Outcome::Value(a), Outcome::Value(b)) if a == b => ConfirmOutcome::ValuesAgree,
        (direct, cadenza) => ConfirmOutcome::Divergence {
            direct: show(direct),
            cadenza: show(cadenza),
        },
    }
}

impl CzDiff {
    /// Compare two outcomes: a `Skip` on EITHER side → `Agree` (non-comparable); both `Trap` → `Agree`;
    /// equal `Value` → `Agree`; anything else (differing values, or one traps + one values) → `Mismatch`.
    fn from_sides(direct: &Outcome, cadenza: &Outcome) -> CzDiff {
        match (direct, cadenza) {
            // A Timeout is a HANG handled by an early return in `cadenza_diff`; if one reaches here treat it
            // as non-comparable (never a false value-mismatch).
            (Outcome::Skip(_) | Outcome::Timeout, _) | (_, Outcome::Skip(_) | Outcome::Timeout) => {
                CzDiff::Agree
            }
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

    /// `classify_confirm` (pure) splits the confirm outcomes: equal values / both-trap → ValuesAgree (a
    /// symbolic false-positive when it confirms an equiv suspected divergence); differing values or a
    /// value-vs-trap split → Divergence; a Skip/Timeout on either side → Uncomparable.
    #[test]
    fn classify_confirm_splits_agree_from_uncomparable() {
        use Outcome::*;
        assert_eq!(
            classify_confirm(&Value("7".into()), &Value("7".into())),
            ConfirmOutcome::ValuesAgree
        );
        assert_eq!(classify_confirm(&Trap, &Trap), ConfirmOutcome::ValuesAgree);
        assert!(matches!(
            classify_confirm(&Value("7".into()), &Value("8".into())),
            ConfirmOutcome::Divergence { .. }
        ));
        assert!(matches!(
            classify_confirm(&Value("7".into()), &Trap),
            ConfirmOutcome::Divergence { .. }
        ));
        // A skip on either side = the sampled net couldn't compare (NOT a symbolic false-positive).
        assert_eq!(
            classify_confirm(&Skip("x".into()), &Value("7".into())),
            ConfirmOutcome::Uncomparable
        );
        assert_eq!(
            classify_confirm(&Value("7".into()), &Skip("y".into())),
            ConfirmOutcome::Uncomparable
        );
    }

    /// Unparseable source yields no equiv trial (SKIP) — and short-circuits BEFORE invoking `cdz` (the
    /// `sexpr::read(...)?` fails first), so this is a pure test needing no binary.
    #[test]
    fn equiv_trial_for_skips_unparseable_source() {
        let tmp = std::env::temp_dir();
        assert!(
            equiv_trial_for(Path::new("/nonexistent-cdz"), "(( not balanced", &tmp).is_none(),
            "unparseable source must skip (None) without touching cdz"
        );
    }

    /// LIVE: a cleanly round-tripping scalar program yields an equiv trial pairing the ORIGINAL AST with
    /// its `--target cadenza` round-trip — both self-contained programs (a `(do …)` list root). Skips
    /// (does not fail) when no `cdz` is discoverable; the fuzz cycle runs it for real.
    #[test]
    fn equiv_trial_for_pairs_orig_with_cadenza_roundtrip() {
        let Some(cdz) = crate::differential::discover_cdz() else {
            eprintln!("skipping: no cdz binary discovered (set CDZ_SMITH_CDZ)");
            return;
        };
        let tmp =
            std::env::temp_dir().join(format!("cdz-smith-equiv-trial-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let src = "(do (def (main) (+ 1 2)) (export main))";
        match equiv_trial_for(&cdz, src, &tmp) {
            Some(t) => {
                use cadenza_syntax::ast::Struct;
                assert!(
                    matches!(t.orig.get(t.orig.root), Struct::List(_)),
                    "orig side is a program"
                );
                assert!(
                    matches!(t.cadenza.get(t.cadenza.root), Struct::List(_)),
                    "cadenza round-trip side is a program"
                );
            }
            None => eprintln!("skipping assert: cadenza round-trip declined for this cdz build"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// LIVE regression tripwire (#8243 — the cdz-smith S298/S301 finding): the `--target cadenza`
    /// Core→surface re-emit must PRESERVE a sized-int (Int8/Int16/Int32/UInt8/…) width on a container
    /// ELEMENT (and Map key/value). Before #8243 the ConstInt ascription only fired for unsigned /
    /// out-of-i64-range, so a bare non-default-width element re-grounded to the default Int64 and a
    /// container round-tripped with widened element types (`(Tuple Int8 Int32)` → `(Tuple Int64 Int64)`)
    /// — value-correct but type-width-wrong. #8243 broadened the guard to `ground_width() != 64`. Each
    /// form below must round-trip WITHOUT a `ConfirmOutcome::Divergence`. Skips (does not fail) when no
    /// `cdz` is discoverable, and treats Uncomparable/Hang as a skip (never a false regression); the fuzz
    /// cycle + a local build run it for real.
    #[test]
    fn sized_int_width_preserved_in_containers_reemit() {
        let Some(cdz) = crate::differential::discover_cdz() else {
            eprintln!("skipping: no cdz binary discovered (set CDZ_SMITH_CDZ)");
            return;
        };
        let base = std::env::temp_dir().join(format!(
            "cdz-smith-sized-int-tripwire-{}",
            std::process::id()
        ));
        let tmp = base.join("tmp");
        let store = base.join("store");
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&store);
        // One form per container kind, each with a non-default-width sized-int element (Map: key+value).
        let cases: &[(&str, &str)] = &[
            (
                "tuple",
                "(do (def (main) (tuple (: 6 Int8) (: 7 Int32))) (export main))",
            ),
            (
                "record",
                "(do (def (main) (do (def (g (: x (Record (: a Bool) (: b Int8)))) x) (g (record (= a true) (= b (: 8 Int8)))))) (export main))",
            ),
            (
                "list",
                "(do (def (main) (list (: 6 Int8) (: 7 Int8))) (export main))",
            ),
            (
                "set",
                "(do (def (main) (Set.of (list (: 6 Int8) (: 7 Int8)))) (export main))",
            ),
            (
                "map",
                "(do (def (main) (Map.insert (Map.empty) (: 6 Int8) (: 7 Int8))) (export main))",
            ),
        ];
        let mut ran = 0usize;
        for (name, src) in cases {
            match cadenza_confirm(&cdz, src, &store, &tmp) {
                ConfirmOutcome::Divergence { direct, cadenza } => panic!(
                    "#8243 REGRESSION: container `{name}` sized-int element re-emit diverged — \
                     direct={direct} cadenza={cadenza}"
                ),
                // ValuesAgree = fix holds; Uncomparable/Hang = this cdz build couldn't run the case
                // (capability gap / no cdz-run) — a skip, never a false regression.
                ConfirmOutcome::ValuesAgree => ran += 1,
                _ => {}
            }
        }
        if ran == 0 {
            eprintln!("skipping assert: no case was runnable on this cdz build (all uncomparable)");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
