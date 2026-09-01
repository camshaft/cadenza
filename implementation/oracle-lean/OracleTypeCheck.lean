/-
`oracle-typecheck` — the TYPE-oracle corpus-conformance runner (T0.2). Given a corpus case's
`program.ast`, run the type oracle's `infer` and judge it against rcdzc's verdict.

rcdzc's verdict per case is DERIVED from the corpus expectation in `oracle-trial.ast` (BIDIRECTIONAL —
the manifest holds both accept cases AND the negative type-system cases whose expected outcome is a
compile error):
  * `Expect.value` / `Expect.trap`  → rcdzc `accept`  (the program compiled + ran)
  * `Expect.error code`             → rcdzc `reject code` (rcdzc rejected it at compile — a type error)
  * `Expect.declines`               → rcdzc `decline`
Then `judgeTypecheck (infer prog) rcdzcVerdict` (design §1.2). A `mismatch` fails the run:
  * `WellTyped` vs a coded reject  → FALSE-REJECT (over-strict rcdzc, the operator's #1 direction — or an
                                     oracle bug that fails to model the rejection)
  * `IllTyped`  vs accept          → FALSE-ACCEPT (soundness hole, or an over-strict oracle rule)
`skip` (`Unsupported` = outside the modeled fragment) never fails (monotone coverage). This is the corpus
quality signal for the type oracle, mirroring `oracle-check` for the semantics oracle.

Two modes:
  oracle-typecheck <program.ast> <oracle-trial.ast>   — one case
  oracle-typecheck --manifest <FILE>                  — FILE lists case DIRS, aggregated
-/
import Oracle.Type
import Oracle.Check

open Oracle

structure TTally where
  holds : Nat := 0
  mismatch : Nat := 0
  skip : Nat := 0
  deriving Inhabited

def TTally.add (a b : TTally) : TTally :=
  { holds := a.holds + b.holds, mismatch := a.mismatch + b.mismatch, skip := a.skip + b.skip }

/-- The rcdzc verdict a corpus case carries, derived from its trials' expected outcomes: any trial that
expects a compile `error` ⇒ `reject` (a type-rejection case); a `declines` ⇒ `decline`; otherwise (all
`value`/`trap`) ⇒ `accept`. -/
def rcdzcVerdictOf (trials : Array OTrial) : RcdzcVerdict :=
  match trials.findSome? (fun t => match t.expect with
                                   | .error c => some (RcdzcVerdict.reject c)
                                   | .declines => some RcdzcVerdict.decline
                                   | _ => none) with
  | some v => v
  | none => .accept

/-- Run one case: decode `program.ast` + `oracle-trial.ast`, `infer`, judge against the trial-derived verdict. -/
def checkCase (progPath otPath : String) (skips : Bool := false) : IO TTally := do
  let progBytes ← IO.FS.readBinFile progPath
  let otBytes ← IO.FS.readBinFile otPath
  match Ast.decode progBytes, Ast.decode otBytes with
  | .error e, _ => IO.eprintln s!"decode {progPath}: {e}"; return { mismatch := 1 }
  | _, .error e => IO.eprintln s!"decode {otPath}: {e}"; return { mismatch := 1 }
  | .ok prog, .ok ot =>
    match Check.parseTrials ot with
    -- a trial shape we can't parse (e.g. a warning-only case with no expect-clause) → SKIP: we can't derive
    -- rcdzc's verdict, so we can't judge (a sound coverage gap, never a spurious mismatch).
    | .error _ => return { skip := 1 }
    | .ok trials =>
      match judgeTypecheck (infer prog) (rcdzcVerdictOf trials) with
      | .holds => return { holds := 1 }
      | .skip r =>
        if skips then IO.eprintln s!"SKIP {progPath}: {r}"
        return { skip := 1 }
      | .mismatch d => IO.println s!"MISMATCH {progPath}: {d}"; return { mismatch := 1 }  -- stdout: visible in the gate log (nix -L truncates stderr)

/-- Manifest lines → case dirs (newline-separated; trimmed, blanks dropped). -/
def readDirs (file : String) : IO (List String) := do
  let text ← IO.FS.readFile file
  return (text.splitOn "\n").map String.trim |>.filter (· ≠ "")

def main (args : List String) : IO UInt32 := do
  let skips := args.contains "--skips"
  let args := args.filter (· != "--skips")
  let mut total : TTally := {}
  match args with
  | ["--manifest", file] =>
    let dirs ← readDirs file
    for d in dirs do
      total := total.add (← checkCase (d ++ "/program.ast") (d ++ "/oracle-trial.ast") skips)
  | [prog, ot] =>
    total ← checkCase prog ot skips
  | _ =>
    IO.eprintln "oracle-typecheck: usage: oracle-typecheck [--skips] (<program.ast> <oracle-trial.ast> | --manifest FILE)"
    return 2
  IO.println s!"oracle-typecheck: {total.holds} holds, {total.mismatch} mismatch, {total.skip} skip"
  return (if total.mismatch == 0 then 0 else 1)
