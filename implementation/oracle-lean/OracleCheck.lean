/-
`oracle-check` — the corpus-conformance / assertion runner (L1.2). Given a case's `program.ast` and
`oracle-trial.ast`, the oracle evaluates each trial and ASSERTS the expected outcome, reporting a
per-trial verdict. A `mismatch` on ANY realized trial is a real oracle-vs-corpus disagreement (fails);
`skip` (Unsupported/Diverges/compile-outcome) is a sound coverage-gap, never a failure.

Two modes:
  oracle-check <program.ast> <oracle-trial.ast>   — one case (the flake per-case exec form)
  oracle-check --manifest <FILE>                  — FILE lists case DIRS (each with program.ast +
                                                    oracle-trial.ast), aggregated (local corpus run)
Exit non-zero iff any trial mismatches.
-/
import Oracle

open Oracle

structure Tally where
  holds : Nat := 0
  mismatch : Nat := 0
  skip : Nat := 0
  deriving Inhabited

/-- Run one case's assertions, printing any mismatch. Returns the trial tally. -/
def checkCase (progPath otPath : String) : IO Tally := do
  let progBytes ← IO.FS.readBinFile progPath
  let otBytes ← IO.FS.readBinFile otPath
  match Ast.decode progBytes, Ast.decode otBytes with
  | .error e, _ => IO.eprintln s!"decode {progPath}: {e}"; return { mismatch := 1 }
  | _, .error e => IO.eprintln s!"decode {otPath}: {e}"; return { mismatch := 1 }
  | .ok prog, .ok ot =>
    match Check.check prog ot with
    | .error e => IO.eprintln s!"parse {otPath}: {e}"; return { mismatch := 1 }
    | .ok verdicts =>
      let mut t : Tally := {}
      for v in verdicts do
        match v with
        | .holds => t := { t with holds := t.holds + 1 }
        | .skip _ => t := { t with skip := t.skip + 1 }
        | .mismatch d =>
          t := { t with mismatch := t.mismatch + 1 }
          IO.eprintln s!"MISMATCH {progPath}: {d}"
      return t

/-- Manifest lines → case dirs (newline-separated; trimmed, blanks dropped). -/
def readDirs (file : String) : IO (List String) := do
  let text ← IO.FS.readFile file
  return (text.splitOn "\n").map String.trim |>.filter (· ≠ "")

/-- Merge two tallies. -/
def Tally.add (a b : Tally) : Tally :=
  { holds := a.holds + b.holds, mismatch := a.mismatch + b.mismatch, skip := a.skip + b.skip }

def main (args : List String) : IO UInt32 := do
  let mut total : Tally := {}
  match args with
  | ["--manifest", file] =>
    let dirs ← readDirs file
    for d in dirs do
      total := total.add (← checkCase (d ++ "/program.ast") (d ++ "/oracle-trial.ast"))
  | [prog, ot] =>
    total ← checkCase prog ot
  | _ =>
    IO.eprintln "oracle-check: usage: oracle-check (<program.ast> <oracle-trial.ast> | --manifest FILE)"
    return 2
  IO.println s!"oracle-check: {total.holds} holds, {total.mismatch} mismatch, {total.skip} skip"
  return (if total.mismatch == 0 then 0 else 1)
