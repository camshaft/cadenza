/-
L2 batch differential (design §L2): the async 3rd-Side interface with the fuzzer (`v-cdz-smith`).

The fuzzer runs a program under rcdzc, captures rcdzc's OUTPUT, and hands the oracle a batch of trials
`(trial <program-ast> (args <v>…)? <rcdzc-output>)` where `<rcdzc-output> = (value <ast-value>) | (trap
"<reason>")`. The oracle re-derives the output (`execute`/`reduce`) and ASSERTS it matches rcdzc's — a
per-trial `holds`/`mismatch`/`skip` (reusing `Check.checkTrial`: value byte-compared, trap compared by
canonical kind). A `mismatch` is a candidate rcdzc bug.

Everything crosses as binary AST + uleb128 frames (`Oracle.Leb`), so the fuzzer can pipeline: compile
batch N+1 while the oracle judges batch N.

REQUEST batch frame:  uleb `n`, then n × (uleb `len` + `len` cdzast bytes)   [each blob = one trial module]
VERDICT batch frame:  uleb `n`, then n × verdict, where a verdict is
  0x00                                         — holds
  0x01, uleb `len`, `len` UTF-8 bytes          — mismatch (detail text; my computed output vs rcdzc's)
  0x02, uleb `len`, `len` UTF-8 bytes          — skip (reason)
-/
import Oracle.Leb
import Oracle.Ast
import Oracle.Eval
import Oracle.Check

namespace Oracle
open Oracle.Ast

namespace Batch

/-- The child node ids of node `cid` (after its head), safely (`#[]` if it is not a list / out of range). -/
def kidsOf (m : Module) (cid : Nat) : Array Nat := ((m.nodes[cid]?).map Check.argsOf).getD #[]

/-- Parse a trial blob module `(trial <program> (args <v>…)? <output>)` into a program module (the blob
module re-rooted at the program node, so `execute` runs it) and a `Check.OTrial` (arg value-AST node ids
+ the expected outcome, both referencing the blob module). Output `(value <v>)` → expect that value;
`(trap "<reason>")` → expect that trap. -/
def parseTrialModule (m : Module) : Except String (Module × OTrial) := do
  if Check.headStr? m m.root != some "trial" then
    .error "batch: blob root is not (trial …)"
  else
    let rootKids := kidsOf m m.root
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

/-- Judge one trial blob → a `Verdict`. An undecodable / malformed blob is a `skip` (never crashes the
batch): a garbled trial is a coverage-gap, not a differential mismatch. -/
def judgeBlob (blob : ByteArray) : Verdict :=
  match Ast.decode blob with
  | .error e => .skip s!"batch: undecodable trial blob ({e})"
  | .ok m =>
    match parseTrialModule m with
    | .error e => .skip s!"batch: {e}"
    | .ok (prog, t) => Check.checkTrial prog m t

/-- Decode a REQUEST batch frame into its trial blobs. -/
def decodeBatch (bytes : ByteArray) : Except String (Array ByteArray) := do
  let (n, c) ← (Leb.Cursor.ofBytes bytes).readUleb
  let rec go (c : Leb.Cursor) (k : Nat) (acc : Array ByteArray) : Except String (Array ByteArray) :=
    match k with
    | 0 => .ok acc
    | k + 1 => do
      let (len, c) ← c.readUleb
      let (blob, c) ← c.readBytes len
      go c k (acc.push blob)
  go c n #[]

/-- Encode verdicts into a VERDICT batch frame. -/
def encodeVerdicts (vs : Array Verdict) : ByteArray :=
  vs.foldl (fun acc v =>
    match v with
    | .holds => acc.push 0x00
    | .mismatch d => let b := d.toUTF8; ((acc.push 0x01) ++ Leb.encode b.size) ++ b
    | .skip r => let b := r.toUTF8; ((acc.push 0x02) ++ Leb.encode r.toUTF8.size) ++ b)
    (Leb.encode vs.size)

/-- Judge a whole REQUEST batch frame → its VERDICT batch frame. -/
def judgeBatchBytes (bytes : ByteArray) : Except String ByteArray := do
  let blobs ← decodeBatch bytes
  pure (encodeVerdicts (blobs.map judgeBlob))

-- ── compile-time self-tests (gated by `nix build .#oracle-lean`) ───────────────────────────────────

-- Frame round-trip: a 2-blob request frame decodes to exactly those blobs.
private def _frame : ByteArray :=
  ((Leb.encode 2 ++ Leb.encode 3) ++ ByteArray.mk #[1, 2, 3] ++ Leb.encode 1) ++ ByteArray.mk #[9]
#guard (match decodeBatch _frame with
        | .ok bs => bs.size == 2 && bs[0]! == ByteArray.mk #[1, 2, 3] && bs[1]! == ByteArray.mk #[9]
        | _ => false)

-- Verdict framing: holds encodes as `uleb 1` then the 0x00 tag.
#guard (encodeVerdicts #[Verdict.holds] == ByteArray.mk #[1, 0])

-- A malformed blob is a skip, not a crash.
#guard (match judgeBlob (ByteArray.mk #[0, 1, 2]) with | .skip _ => true | _ => false)

-- End-to-end: `(trial (do (def (main) 42) (export main)) (args) (value 42))` → the oracle re-derives 42
-- and it matches rcdzc's 42 → `holds`. Exercises decode + parseTrialModule + re-root + checkTrial. NOTE:
-- the AST is a TREE (each NODE referenced once), so `main`/`42` get duplicate ATOM nodes (n3/n6, n4/n9).
private def _trialHolds : Module :=
  { leaves := #[Leaf.name "trial".toUTF8, Leaf.name "do".toUTF8, Leaf.name "def".toUTF8,
                Leaf.name "main".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[42]),
                Leaf.name "export".toUTF8, Leaf.name "args".toUTF8, Leaf.name "value".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .atom 3, .atom 4, .atom 5, .atom 3, .atom 6, .atom 7, .atom 4,
               .list #[3], .list #[2, 10, 4], .list #[5, 6], .list #[1, 11, 12], .list #[7],
               .list #[8, 9], .list #[0, 13, 14, 15]],
    root := 16 }
#guard (match judgeBlob (Ast.encode _trialHolds) with | .holds => true | _ => false)

end Batch
end Oracle
