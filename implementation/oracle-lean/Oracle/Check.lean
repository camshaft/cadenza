/-
The oracle as an ASSERTION CHECKER (operator architecture, design §1): given a program (`program.ast`)
and its trials-with-expected-outcomes (`oracle-trial.ast`, the frozen artifact the corpus shred emits),
evaluate each trial and ASSERT INTERNALLY whether the expected outcome holds — returning a per-trial
VERDICT, not a raw value for a caller to compare. Everything is binary AST; no s-expr is parsed.

Comparison is over the oracle's own value domain: the expected value-AST is interpreted (its `(: v T)`
frame stripped to the value `v`; type-aware alias-expanding comparison is a later refinement — for the
scalar core, comparing the underlying value matches how the corpus grades, which strips the ascription).
An `Unsupported`/`Diverges` computation, or an `expect-error`/`expect-declines` (a COMPILE outcome the
evaluator does not model), is a SKIP — a sound coverage-gap, never a mismatch (design §1.2).
-/
import Oracle.Ast
import Oracle.Value
import Oracle.Eval

namespace Oracle

open Oracle.Ast

/-- A trial's expected outcome, as parsed from `oracle-trial.ast`. `value` holds the node id of the
expected value-AST (within the oracle-trial module); the rest are the run/compile outcome kinds. -/
inductive Expect where
  | value (node : Nat)
  | trap (kind : String)
  | error (code : String)
  | declines
  deriving Inhabited

/-- One parsed trial: an optional call export, its argument value-AST node ids, and the expectation. -/
structure OTrial where
  call : Option String := none
  args : Array Nat := #[]
  expect : Expect
  deriving Inhabited

/-- The verdict for a trial. `skip` is a sound coverage-gap (never a differential mismatch). -/
inductive Verdict where
  | holds
  | mismatch (detail : String)
  | skip (reason : String)
  deriving Inhabited, BEq

namespace Check

/-- A `List` node's children after the head (the `(head child…)` convention), or `#[]`. -/
def argsOf (node : Node) : Array Nat :=
  match node with
  | .list cs => if cs.size ≥ 1 then cs.extract 1 cs.size else #[]
  | _ => #[]

/-- The head name (as a `String`) of a `List` node, if any. -/
def headStr? (m : Module) (i : Nat) : Option String :=
  match m.nodes[i]? with
  | some node => (m.headName? node).bind (fun b => String.fromUTF8? b)
  | none => none

/-- The bare-name atom's text, if node `i` is one. -/
def atomName? (m : Module) (i : Nat) : Option String :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) => String.fromUTF8? b
    | _ => none
  | _ => none

/-- A name-or-string-leaf atom's text, if node `i` is one. -/
def leafText? (m : Module) (i : Nat) : Option String :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.str b) => String.fromUTF8? b
    | some (Leaf.name b) => String.fromUTF8? b
    | _ => none
  | _ => none

/-- The text of the FIRST child of a `(head <str-leaf>)` node `i`, or "". -/
def leafTextOf (m : Module) (i : Nat) : String :=
  match m.nodes[i]? with
  | some node => match (argsOf node)[0]? with
                 | some v => (leafText? m v).getD ""
                 | none => ""
  | none => ""

/-- Strip a `(: <value> <type>)` frame to the value node id; a non-framed node is returned as-is. -/
def stripFrame (m : Module) (i : Nat) : Nat :=
  match m.nodes[i]? with
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h => if h == ":".toUTF8 && cs.size ≥ 2 then cs[1]! else i
    | none => i
  | _ => i

mutual
/-- Interpret an expected value-AST node (its `(: v T)` frame stripped) as a `Value` — a scalar leaf,
or a compound `(Some e)` / `(None …)` / `(Ok e)` / `(Err e)` / `(tuple e…)` / `(list e…)` (recursively).
`none` if it is not a value the domain models. -/
partial def expectedValue? (m : Module) (i : Nat) : Option Value :=
  let s := stripFrame m i
  match m.nodes[s]? with
  | Option.some (Node.atom lid) => (m.leaves[lid]?).bind Value.ofLeaf
  | Option.some (Node.list cs) =>
    if Eval.qualHead? m cs == Option.some ("Set".toUTF8, "of".toUTF8) then
      -- a Set value `((. Set of) (list e…))` — parse the list arg, canonicalize (sort + dedupe)
      match (cs[1]?).bind (expectedValue? m) with
      | Option.some (Value.list elems) => (Eval.canonSet elems).map Value.set
      | _ => Option.none
    else
    -- an `Ast` variant VALUE `((. Ast Ctor) payload)` — the qualified-only ctor form a quoted/reflected
    -- program renders as. Parses to the SAME `variant` the eval side's `quoteReflect`/`(. Ast Ctor)`
    -- construction produces, so a quote result compares equal to its written-out `Ast.*` expected form.
    match Eval.qualHead? m cs with
    | Option.some (q, c) =>
      if q == "Ast".toUTF8 then ((cs[1]?).bind (expectedValue? m)).map (fun p => Value.variant c p)
      else Option.none
    | Option.none =>
    match headStr? m s with
    | Option.some "Some" => ((cs[1]?).bind (expectedValue? m)).map Value.some
    | Option.some "Ok" => ((cs[1]?).bind (expectedValue? m)).map Value.ok
    | Option.some "Err" => ((cs[1]?).bind (expectedValue? m)).map Value.err
    | Option.some "None" => Option.some Value.none
    | Option.some "tuple" => (seqVals m (cs.extract 1 cs.size)).map Value.tuple
    | Option.some "list" => (seqVals m (cs.extract 1 cs.size)).map Value.list
    | Option.some "map" => (mapEntries m (cs.extract 1 cs.size)).bind (fun es => (Eval.canonMap es).map Value.map)
    | Option.some "record" => Option.none   -- record output not modeled here (compared via `=` only)
    | Option.some ctor =>
      -- a prelude/user sum VARIANT value `(Ctor payload)` — bare ctor head + one payload child
      -- (nullary renders `(Ctor unit)`); every other name-headed value node is a variant in canonical form
      ((cs[1]?).bind (expectedValue? m)).map (fun p => Value.variant ctor.toUTF8 p)
    | Option.none => Option.none
  | _ => Option.none

/-- Interpret each node as a value; `none` if any element is not a modeled value. -/
partial def seqVals (m : Module) (ids : Array Nat) : Option (Array Value) :=
  ids.foldl (fun acc id =>
    match acc, expectedValue? m id with
    | Option.some vs, Option.some v => Option.some (vs.push v)
    | _, _ => Option.none) (Option.some #[])

/-- Interpret map ENTRY nodes — each a raw positional `(k v)` (current corpus) or a `(= k v)` field-pair
(dual-read, the settled target form) — as (key, value) pairs; `none` if any entry is malformed/unmodeled. -/
partial def mapEntries (m : Module) (ids : Array Nat) : Option (Array (Value × Value)) :=
  ids.foldl (fun acc id =>
    match acc with
    | Option.none => Option.none
    | Option.some es =>
      match m.nodes[id]? with
      | Option.some (Node.list ec) =>
        let (kId, vId) := match m.headName? (Node.list ec) with
          | Option.some h => if h == "=".toUTF8 && ec.size == 3 then (ec[1]?, ec[2]?) else (ec[0]?, ec[1]?)
          | Option.none => (ec[0]?, ec[1]?)
        match kId.bind (expectedValue? m), vId.bind (expectedValue? m) with
        | Option.some k, Option.some v => Option.some (es.push (k, v))
        | _, _ => Option.none
      | _ => Option.none) (Option.some #[])
end

/-- Parse one `(trial (call <export>)? (arg <val>)* (expect-… …))`. -/
def parseTrial (m : Module) (tid : Nat) : Except String OTrial := do
  let kids := match m.nodes[tid]? with | some n => argsOf n | none => #[]
  let mut call : Option String := none
  let mut args : Array Nat := #[]
  let mut expect : Option Expect := none
  for cid in kids do
    match headStr? m cid with
    | some "call" =>
      let cc := match m.nodes[cid]? with | some n => argsOf n | none => #[]
      call := (cc[0]?).bind (fun v => atomName? m v <|> leafText? m v)
    | some "arg" =>
      let cc := match m.nodes[cid]? with | some n => argsOf n | none => #[]
      if let some v := cc[0]? then args := args.push v
    | some "expect-value" =>
      let cc := match m.nodes[cid]? with | some n => argsOf n | none => #[]
      if let some v := cc[0]? then expect := some (.value v)
    | some "expect-trap" => expect := some (.trap (leafTextOf m cid))
    | some "expect-error" => expect := some (.error (leafTextOf m cid))
    | some "expect-declines" => expect := some .declines
    | _ => pure ()
  match expect with
  | some e => .ok { call, args, expect := e }
  | none => .error "oracle-trial: trial has no expect-… clause"

/-- Parse the `oracle-trials` module into its trials. Root: `(oracle-trials (trials (trial …)…) …)`. -/
def parseTrials (m : Module) : Except String (Array OTrial) := do
  match m.nodes[m.root]? with
  | some root =>
    if headStr? m m.root != some "oracle-trials" then
      .error "oracle-trial: root is not (oracle-trials …)"
    else
      let topKids := argsOf root
      match topKids.find? (fun c => headStr? m c == some "trials") with
      | some trialsId =>
        let trialNodes := match m.nodes[trialsId]? with | some n => argsOf n | none => #[]
        trialNodes.foldlM (fun acc tid => do pure (acc.push (← parseTrial m tid))) #[]
      | none => .error "oracle-trial: no (trials …) section"
  | none => .error "oracle-trial: empty module"

/-- Canonicalize a trap reason to its KIND — replicating `cdz-corpus-grade::trap_kind` so a `(trap …)`
comparison is by kind, not the varying reason string (design §1.2). `none` = an uncanonicalized
(custom `trap("…")`) reason. -/
def contains (haystack needle : String) : Bool := (haystack.splitOn needle).length > 1

def trapKind (reason : String) : Option String :=
  let r := reason.toLower
  -- from_id: the 4 STABLE trap-code ids (new corpus form, #4416) resolve directly …
  if r == "div-by-zero" then some "div-by-zero"
  else if r == "out-of-bounds" then some "out-of-bounds"
  else if r == "overflow" then some "overflow"
  else if r == "unreachable" then some "unreachable"
  -- … else classify() a legacy English reason (case-insensitive substring — mirrors
  -- cdz-corpus-grade::classify, the one place English is matched). This from_id-then-classify keeps
  -- both the code-id `(trap "div-by-zero")` and the legacy `(trap "divide by zero")` forms resolving
  -- to the same kind, matching the authoritative grader exactly.
  else if contains r "divide by zero" || contains r "division by zero" || contains r "remainder by zero" then
    some "div-by-zero"
  else if contains r "out of bounds" then some "out-of-bounds"
  else if contains r "overflow" then some "overflow"
  else if contains r "unreachable" || contains r "shift count out of range" then some "unreachable"
  else none

/-- Assert one trial against the program: run it, compare the outcome to the expectation. -/
def checkTrial (prog : Module) (ot : Module) (t : OTrial) : Verdict :=
  -- A no-argument trial is `reduce`; an argument-bearing call binds the arg VALUES to main's params
  -- via `execute`. Args are the trial's value-AST nodes (bare scalar values); a compound/non-scalar
  -- arg the value domain doesn't model yet → skip (not every arg decodes).
  let outcome :=
    if t.args.isEmpty then Oracle.reduce prog
    else
      let argVals := t.args.filterMap (expectedValue? ot)
      if argVals.size == t.args.size then Oracle.execute prog argVals
      else .unsupported "execute: an argument is not a modeled scalar value"
  match t.expect with
  | .error _ | .declines => .skip "expect is a compile outcome (error/declines) — not modeled"
  | .trap kind =>
    match outcome with
    | .trap k =>
      match trapKind kind with
      | some ek => if trapKind k == some ek then .holds
                   else .mismatch s!"expected trap kind {ek}, got trap {k}"
      | none => .skip s!"expected trap reason {kind} has no canonical kind (custom trap) — not modeled"
    | .value _ => .mismatch s!"expected trap {kind}, got a value"
    | .unsupported r => .skip r
    | .diverges => .skip "diverges"
  | .value node =>
    match outcome with
    | .value v =>
      match expectedValue? ot node with
      | some ev => if Value.valueEqSpec v ev then .holds else .mismatch "value mismatch"
      | none => .skip "expected value is not a modeled scalar (compound/typed value)"
    | .trap k => .mismatch s!"expected a value, got trap {k}"
    | .unsupported r => .skip r
    | .diverges => .skip "diverges"

/-- Assert every trial; returns the verdicts in order. -/
def check (prog : Module) (ot : Module) : Except String (Array Verdict) := do
  let trials ← parseTrials ot
  pure (trials.map (checkTrial prog ot))

end Check

end Oracle
