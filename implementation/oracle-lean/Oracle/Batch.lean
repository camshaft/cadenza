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
import Oracle.Symbolic
import Oracle.Type

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

/-- Judge an `(equiv <P-ast> <P'-ast>)` trial — the T2 SYMBOLIC-EQUIVALENCE dimension (operator seq-196):
prove the input program `P` and its `--target cadenza` round-trip `P'` functionally equivalent FOR ALL
INPUTS via `Symbolic.equivMain` (symbolic evaluation → canonical normalization → structural equality).
Maps onto the EXISTING verdict protocol so the fuzzer needs no new decoder: PROVEN → `holds`;
CANNOT-PROVE → `skip` carrying the reason (`boundary: …` = incompleteness limit; `normalized-but-different`
= a STRONG suspected cadenza-backend miscompile worth confirming with the sampled differential). Each
operand is the batch module re-rooted at that program subtree. -/
def judgeEquivNode (m : Module) (nodeId : Nat) : Verdict :=
  match (kidsOf m nodeId)[0]?, (kidsOf m nodeId)[1]? with
  | some pId, some p'Id =>
    match equivMain { m with root := pId } { m with root := p'Id } with
    | .proven => .holds
    | .cannotProve r => .skip s!"equiv: {r}"
  | _, _ => .skip "equiv: missing program operand"

/-- Decode a carried rcdzc `cdz check` verdict node (`(accept)` | `(reject "<CODE>")` | `(decline)`) into a
`RcdzcVerdict` (design §1.3). `none` = malformed (→ the typecheck item skips). -/
def decodeRcdzcVerdict (m : Module) (nodeId : Nat) : Option RcdzcVerdict :=
  match Check.headStr? m nodeId with
  | some "accept" => some .accept
  | some "decline" => some .decline
  | some "reject" => (((kidsOf m nodeId)[0]?).bind (fun c => Check.leafText? m c)).map RcdzcVerdict.reject
  | _ => none

/-- Judge a `(typecheck <program> <rcdzc-verdict>)` item — the TYPING dimension
(DESIGN-lean-type-system-oracle). Runs `Oracle.infer` on the program subtree and maps its `TypeVerdict`
against rcdzc's carried accept/reject/decline via `judgeTypecheck` (§1.2). T0.1: `infer` is all-declining
→ every typecheck item is `skip` (the additive baseline starts all-skip). Reuses the existing
holds/mismatch/skip protocol, so the batch may freely MIX trial/equiv/typecheck items. -/
def judgeTypecheckNode (m : Module) (nodeId : Nat) : Verdict :=
  match (kidsOf m nodeId)[0]?, (kidsOf m nodeId)[1]? with
  | some pId, some rvId =>
    match decodeRcdzcVerdict m rvId with
    | some rv => judgeTypecheck (infer { m with root := pId }) rv
    | none => .skip "typecheck: malformed rcdzc verdict"
  | _, _ => .skip "typecheck: missing program/verdict operand"

/-- Judge one trial NODE within the batch module → a `Verdict`. A `(typecheck …)` node routes to the TYPING
judge; an `(equiv …)` node to the symbolic-equivalence judge; a `(trial …)` node to the value/trap
differential. A malformed trial is a `skip` (never crashes the batch): a garbled trial is a coverage-gap,
not a differential mismatch. -/
def judgeTrialNode (m : Module) (trialId : Nat) : Verdict :=
  if Check.headStr? m trialId == some "typecheck" then judgeTypecheckNode m trialId
  else if Check.headStr? m trialId == some "equiv" then judgeEquivNode m trialId
  else match parseTrialAt m trialId with
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

-- `(equiv PA PB)` where PA = `(do (def (main) na) (export main))`, PB the same with `nb` — two nullary-main
-- programs whose bodies are the literals na/nb. The equiv node's args are the two program subtrees (nodes
-- 9 and 19); the wrapper `(equiv …)` is node 21. Judged by symbolic equivalence: identical bodies → PROVEN
-- (`holds`); differing constant bodies → CANNOT-PROVE (`skip`).
private def _equivProg (na nb : UInt8) : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[na]), Leaf.name "export".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[nb]), Leaf.name "equiv".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
               .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7],
               .atom 1, .atom 2, .list #[11], .atom 5, .list #[10, 12, 13],
               .atom 4, .atom 2, .list #[15, 16], .atom 0, .list #[18, 14, 17],
               .atom 6, .list #[20, 9, 19]],
    root := 21 }
-- two identical programs are PROVEN equivalent for all inputs → `holds` (routed via judgeTrialNode dispatch).
#guard (match judgeTrialNode (_equivProg 42 42) 21 with | .holds => true | _ => false)
-- two programs with different constant bodies are not proven → `skip` (never a false `holds`).
#guard (match judgeEquivNode (_equivProg 42 43) 21 with | .skip _ => true | _ => false)

-- TYPING: `decodeRcdzcVerdict` extracts a coded reject `(reject "CDZ0203")`.
#guard (decodeRcdzcVerdict
    { leaves := #[Leaf.name "reject".toUTF8, Leaf.str "CDZ0203".toUTF8],
      nodes := #[.atom 0, .atom 1, .list #[0, 1]], root := 2 } 2 == some (.reject "CDZ0203"))
-- a `(typecheck <program> (accept))` item → `skip` (T0.1 `infer` all-declining), routed via judgeTrialNode.
#guard (match judgeTrialNode
    { leaves := #[Leaf.name "typecheck".toUTF8, Leaf.name "accept".toUTF8, Leaf.name "do".toUTF8],
      nodes := #[.atom 2, .atom 0, .atom 1, .list #[2], .list #[1, 0, 3]], root := 4 } 4 with
  | .skip _ => true | _ => false)

-- END-TO-END: a `(batch (equiv PA PB))` request through the full `judgeBatchBytes` stdin→stdout path that
-- `cdz-oracle` runs (encode → decode → judge → encode the `(verdicts …)` response). Wraps `_equivProg`'s
-- `(equiv …)` (node 21) in a `(batch …)` root (nodes 22/23). Verifies the fuzzer's real invocation shape.
private def _batchEquiv (na nb : UInt8) : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[na]), Leaf.name "export".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[nb]), Leaf.name "equiv".toUTF8, Leaf.name "batch".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
               .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7],
               .atom 1, .atom 2, .list #[11], .atom 5, .list #[10, 12, 13],
               .atom 4, .atom 2, .list #[15, 16], .atom 0, .list #[18, 14, 17],
               .atom 6, .list #[20, 9, 19],
               .atom 7, .list #[22, 21]],
    root := 23 }
-- `(batch (equiv P P))` (identical) → the full pipeline responds `(verdicts (holds))`.
#guard (match judgeBatchBytes (Ast.encode (_batchEquiv 42 42)) with
        | .ok resp =>
          match Ast.decode resp with
          | .ok rm =>
            Check.headStr? rm rm.root == some "verdicts" &&
            (match (kidsOf rm rm.root)[0]? with
             | some vid => Check.headStr? rm vid == some "holds"
             | none => false)
          | _ => false
        | _ => false)
-- `(batch (equiv P42 P43))` (differing) → `(verdicts (skip …))` (a cannot-prove, never a false holds).
#guard (match judgeBatchBytes (Ast.encode (_batchEquiv 42 43)) with
        | .ok resp =>
          match Ast.decode resp with
          | .ok rm =>
            (match (kidsOf rm rm.root)[0]? with
             | some vid => Check.headStr? rm vid == some "skip"
             | none => false)
          | _ => false
        | _ => false)

end Batch
end Oracle
