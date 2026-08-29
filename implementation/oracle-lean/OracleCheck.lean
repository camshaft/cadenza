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
def checkCase (progPath otPath : String) (skips : Bool := false) : IO Tally := do
  let progBytes ← IO.FS.readBinFile progPath
  let otBytes ← IO.FS.readBinFile otPath
  -- a `wit-world.ast` sibling ⇒ the case's exported result crosses a typed WIT ABI (enum/variant
  -- rename/retype) — Check.check downgrades a WIT-crossed would-be mismatch to a sound skip.
  let witWorld ← (System.FilePath.mk (progPath.replace "program.ast" "wit-world.ast")).pathExists
  match Ast.decode progBytes, Ast.decode otBytes with
  | .error e, _ => IO.eprintln s!"decode {progPath}: {e}"; return { mismatch := 1 }
  | _, .error e => IO.eprintln s!"decode {otPath}: {e}"; return { mismatch := 1 }
  | .ok prog, .ok ot =>
    match Check.check prog ot witWorld with
    | .error e => IO.eprintln s!"parse {otPath}: {e}"; return { mismatch := 1 }
    | .ok verdicts =>
      let mut t : Tally := {}
      for v in verdicts do
        match v with
        | .holds => t := { t with holds := t.holds + 1 }
        | .skip r =>
          t := { t with skip := t.skip + 1 }
          if skips then IO.eprintln s!"SKIP {progPath}: {r}"
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

/-- Read every byte available on a stream. -/
partial def readAll (s : IO.FS.Stream) (acc : ByteArray) : IO ByteArray := do
  let chunk ← s.read 65536
  if chunk.isEmpty then return acc else readAll s (acc ++ chunk)

def main (args : List String) : IO UInt32 := do
  -- L2 differential (design §L2): read a REQUEST batch frame on stdin, judge each trial (oracle output
  -- vs the rcdzc output carried in the trial), write the VERDICT batch frame on stdout. A malformed
  -- FRAME is a diagnostic + non-zero exit (never a bogus verdict frame); a malformed TRIAL is a skip.
  if args == ["--batch-stream"] then
    let input ← readAll (← IO.getStdin) ByteArray.empty
    match Batch.judgeBatchBytes input with
    | .error e => (← IO.getStderr).putStrLn s!"oracle-check --batch-stream: {e}"; return 1
    | .ok resp => (← IO.getStdout).write resp; return 0
  -- `--skips` (diagnostic): also print each `skip`'s reason to stderr, for surveying coverage gaps.
  let skips := args.contains "--skips"
  let args := args.filter (· != "--skips")
  let mut total : Tally := {}
  match args with
  | ["--manifest", file] =>
    let dirs ← readDirs file
    for d in dirs do
      total := total.add (← checkCase (d ++ "/program.ast") (d ++ "/oracle-trial.ast") skips)
  | [prog, ot] =>
    total ← checkCase prog ot skips
  | _ =>
    IO.eprintln "oracle-check: usage: oracle-check [--skips] (<program.ast> <oracle-trial.ast> | --manifest FILE)"
    return 2
  IO.println s!"oracle-check: {total.holds} holds, {total.mismatch} mismatch, {total.skip} skip"
  return (if total.mismatch == 0 then 0 else 1)
