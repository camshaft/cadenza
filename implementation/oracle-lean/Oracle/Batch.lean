/-
L2 batch differential (design §L2): the async 3rd-Side interface with the fuzzer (`v-cdz-smith`).

The fuzzer runs a program under rcdzc, captures rcdzc's OUTPUT, and hands the oracle a batch of trials
`(trial <program-ast> (args <v>…)? <rcdzc-output>)` where `<rcdzc-output> = (value <ast-value>) | (trap
"<reason>")`. The oracle re-derives the output (`execute`/`reduce`) and ASSERTS it matches rcdzc's — a
per-trial `holds`/`mismatch`/`skip` (reusing `Check.checkTrial`: value byte-compared, trap compared by
canonical kind). A `mismatch` is a candidate rcdzc bug.

Everything crosses as ONE binary-AST tree — no bespoke frame (operator 2026-08-28: "why aren't we using
the AST? Lean already has an encoder/decoder"). Both sides speak only `Ast.decode`/`Ast.encode`; uleb128
survives only INSIDE the codec. The fuzzer can still pipeline (compile batch N+1 while the oracle judges
batch N) — a batch is one read-all-stdin tree.

REQUEST  = one cdzast blob:  `(batch <trial1> <trial2> …)`  — each `<trialN>` an ordinary `(trial …)` node.
RESPONSE = one cdzast blob:  `(verdicts <v1> <v2> …)`  — one child per trial, in order, each:
  `(holds)`                — the oracle re-derived rcdzc's output
  `(mismatch <detail>)`    — `<detail>` a `str` leaf: my computed output vs rcdzc's (a candidate bug)
  `(skip <reason>)`        — `<reason>` a `str` leaf: a sound coverage-gap (undecodable/unmodeled)
-/
import Oracle.Ast
import Oracle.Eval
import Oracle.Check

namespace Oracle
open Oracle.Ast

namespace Batch

/-- The child node ids of node `cid` (after its head), safely (`#[]` if it is not a list / out of range). -/
def kidsOf (m : Module) (cid : Nat) : Array Nat := ((m.nodes[cid]?).map Check.argsOf).getD #[]

/-- Parse the `(trial <program> (args <v>…)? <output>)` node `trialId` (a subtree of the batch module `m`)
into a program module (the SAME module re-rooted at the program node, so `execute` runs it) and a
`Check.OTrial` (arg value-AST node ids + the expected outcome, both referencing `m`). Output `(value <v>)`
→ expect that value; `(trap "<reason>")` → expect that trap. -/
def parseTrialAt (m : Module) (trialId : Nat) : Except String (Module × OTrial) := do
  if Check.headStr? m trialId != some "trial" then
    .error "batch: node is not (trial …)"
  else
    let rootKids := kidsOf m trialId
    let some progId := rootKids[0]?
      | .error "batch: trial has no program node"
    let args := (rootKids.findSome? (fun cid =>
      if Check.headStr? m cid == some "args" then some (kidsOf m cid) else none)).getD #[]
    let expect? := rootKids.findSome? (fun cid =>
      match Check.headStr? m cid with
      | some "value" => (kidsOf m cid)[0]?.map Expect.value
      | some "trap" => some (Expect.trap (Check.leafTextOf m cid))
      | _ => none)
    match expect? with
    | some e => .ok ({ m with root := progId }, { args := args, expect := e })
    | none => .error "batch: trial has no (value …)/(trap …) output"

/-- Judge one trial NODE within the batch module → a `Verdict`. A malformed trial is a `skip` (never
crashes the batch): a garbled trial is a coverage-gap, not a differential mismatch. -/
def judgeTrialNode (m : Module) (trialId : Nat) : Verdict :=
  match parseTrialAt m trialId with
  | .error e => .skip s!"batch: {e}"
  | .ok (prog, t) => Check.checkTrial prog m t

/-! ### Building the `(verdicts …)` response tree

A tiny bottom-up AST builder threading the `(leaves, nodes)` state — every verdict is a node with a
name-atom head and (for mismatch/skip) a `str` payload atom, gathered under one `(verdicts …)` root. -/

/-- The response builder's state: the leaf pool and node pool accumulated so far. -/
abbrev BS := Array Leaf × Array Node

/-- Push a `name` leaf + its atom node; return the updated state and the atom NODE id. -/
def bAddNameAtom (s : BS) (name : String) : BS × Nat :=
  let lid := s.1.size
  let leaves := s.1.push (Leaf.name name.toUTF8)
  let nid := s.2.size
  ((leaves, s.2.push (Node.atom lid)), nid)

/-- Push a `str` leaf + its atom node; return the updated state and the atom NODE id. -/
def bAddStrAtom (s : BS) (b : ByteArray) : BS × Nat :=
  let lid := s.1.size
  let leaves := s.1.push (Leaf.str b)
  let nid := s.2.size
  ((leaves, s.2.push (Node.atom lid)), nid)

/-- Push a `list` node over the given child node ids; return the updated state and the list NODE id. -/
def bAddList (s : BS) (kids : Array Nat) : BS × Nat :=
  let nid := s.2.size
  ((s.1, s.2.push (Node.list kids)), nid)

/-- Build one verdict node `(holds)` | `(mismatch <detail>)` | `(skip <reason>)`; return its NODE id. -/
def bVerdict (s : BS) (v : Verdict) : BS × Nat :=
  match v with
  | .holds => let (s, h) := bAddNameAtom s "holds"; bAddList s #[h]
  | .mismatch d =>
    let (s, h) := bAddNameAtom s "mismatch"
    let (s, dn) := bAddStrAtom s d.toUTF8
    bAddList s #[h, dn]
  | .skip r =>
    let (s, h) := bAddNameAtom s "skip"
    let (s, rn) := bAddStrAtom s r.toUTF8
    bAddList s #[h, rn]

/-- Build the `(verdicts <v1> …)` response module from the per-trial verdicts (one child per trial, in
order). Both sides speak only binary AST — this is `Ast.encode`d as the whole response. -/
def buildVerdicts (vs : Array Verdict) : Module :=
  let (s, vids) := vs.foldl (fun (acc : BS × Array Nat) v =>
    let (s, nid) := bVerdict acc.1 v
    (s, acc.2.push nid)) (((#[], #[]) : BS), (#[] : Array Nat))
  let (s, hv) := bAddNameAtom s "verdicts"
  let (s, rootId) := bAddList s (#[hv] ++ vids)
  { leaves := s.1, nodes := s.2, root := rootId }

/-- Judge a whole REQUEST `(batch <trial>…)` tree → its RESPONSE `(verdicts <v>…)` tree bytes. An
undecodable request / a non-`(batch …)` root is a hard error (a diagnostic + non-zero exit at the exe
boundary), never a bogus verdict tree; a malformed individual TRIAL is a per-trial `skip`. -/
def judgeBatchBytes (bytes : ByteArray) : Except String ByteArray := do
  let m ← Ast.decode bytes
  if Check.headStr? m m.root != some "batch" then
    .error "batch: request root is not (batch …)"
  else
    let verdicts := (kidsOf m m.root).map (judgeTrialNode m)
    pure (Ast.encode (buildVerdicts verdicts))

-- ── compile-time self-tests (gated by `nix build .#oracle-lean`) ───────────────────────────────────

-- `buildVerdicts` shapes: `(verdicts (holds) (mismatch "x"))` — one child per verdict, in order.
#guard (let m := buildVerdicts #[Verdict.holds, Verdict.mismatch "x"]
        Check.headStr? m m.root == some "verdicts" &&
        (match (kidsOf m m.root)[0]?, (kidsOf m m.root)[1]? with
         | some v0, some v1 => Check.headStr? m v0 == some "holds" && Check.headStr? m v1 == some "mismatch"
         | _, _ => false))

-- A non-`(trial …)` node judges as a `skip`, not a crash.
#guard (match judgeTrialNode { leaves := #[Leaf.name "x".toUTF8], nodes := #[Node.atom 0], root := 0 } 0 with
        | .skip _ => true | _ => false)

-- A non-`(batch …)` request root is a hard error (never a bogus verdict tree).
#guard (match judgeBatchBytes (Ast.encode { leaves := #[Leaf.name "nope".toUTF8], nodes := #[Node.atom 0], root := 0 }) with
        | .error _ => true | _ => false)

-- End-to-end: `(batch (trial (do (def (main) 42) (export main)) (args) (value 42)))` → the oracle
-- re-derives 42, it matches rcdzc's 42 → the response is `(verdicts (holds))`. The trial subtree mirrors
-- the AST TREE shape (each NODE referenced once, so `main`/`42` get duplicate atom nodes n3/n6, n4/n9);
-- nodes 17/18 wrap it in the `(batch …)` root.
private def _batchHolds : Module :=
  { leaves := #[Leaf.name "trial".toUTF8, Leaf.name "do".toUTF8, Leaf.name "def".toUTF8,
                Leaf.name "main".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[42]),
                Leaf.name "export".toUTF8, Leaf.name "args".toUTF8, Leaf.name "value".toUTF8,
                Leaf.name "batch".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .atom 3, .atom 4, .atom 5, .atom 3, .atom 6, .atom 7, .atom 4,
               .list #[3], .list #[2, 10, 4], .list #[5, 6], .list #[1, 11, 12], .list #[7],
               .list #[8, 9], .list #[0, 13, 14, 15], .atom 8, .list #[17, 16]],
    root := 18 }
#guard (match judgeBatchBytes (Ast.encode _batchHolds) with
        | .ok resp =>
          match Ast.decode resp with
          | .ok rm =>
            Check.headStr? rm rm.root == some "verdicts" &&
            (match (kidsOf rm rm.root)[0]? with
             | some vid => Check.headStr? rm vid == some "holds"
             | none => false)
          | _ => false
        | _ => false)

end Batch
end Oracle
