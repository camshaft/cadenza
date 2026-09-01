/-
`Oracle/Type.lean` — the Lean TYPE-SYSTEM oracle (design: DESIGN-lean-type-system-oracle.md).

The COMPLEMENT to the semantics oracle: an independent typing judgment that validates rcdzc's ACCEPT/REJECT
decision. The fuzzer drives it on rcdzc's REJECTED programs — a Lean-ACCEPTS over a coded reject is a
FALSE-REJECT (an over-strict compiler bug), the false-reject blind spot the wasm-vs-rust differential can
never see (a shared-frontend decline makes both backends agree). "Oracles in both directions."

This file is Phase T0.1: the verdict algebra (§1.1), the all-declining `infer` (HM inference lands at T1),
and `judgeTypecheck` — the §1.2 differential classification mapping a `(TypeVerdict, RcdzcVerdict)` pair onto
the EXISTING `holds`/`mismatch`/`skip` protocol (so the fuzzer↔Lean wire needs no verdict-protocol change).
-/
import Oracle.Ast
import Oracle.Check
import Oracle.Eval

namespace Oracle

/-- A Cadenza type over the modeled subset (`spec/capabilities/type-system.md`). Minimal HM skeleton for
the T0.1 declining stage; extended (rows, sums, units, effects) as inference lands at T1+. -/
inductive Ty where
  | int (width : Nat) (signed : Bool)   -- fixed-width integers (I8…I64/U8…U64)
  | bool | unit | string | char
  | fn (dom cod : Ty)                   -- curried function
  | tuple (elts : List Ty)
  | var (id : Nat)                      -- a unification (type) variable
  deriving BEq, Inhabited

/-- A CDZ diagnostic code (e.g. `"CDZ0203"`), carried on a coded reject. -/
abbrev Code := String

/-- The oracle's typing verdict (design §1.1): a pure, total function's output. -/
inductive TypeVerdict where
  | wellTyped (τ : Ty)            -- accepts; `τ` = the principal type (compared only at T4)
  | illTyped (code : Code)        -- rejects with a specific CDZ diagnostic code
  | unsupported (reason : String) -- the oracle declines to model this program (a SOUND coverage gap)
  deriving Inhabited, BEq

/-- rcdzc's frontend `cdz check` verdict, carried in the `(typecheck …)` batch item (design §1.2/§1.3):
`accept` / `reject(code)` (a CODED error-severity fault) / `decline` (a CODELESS "not yet implemented"). -/
inductive RcdzcVerdict where
  | accept
  | reject (code : Code)
  | decline
  deriving BEq, Inhabited

/-- The type of a SCALAR LITERAL node (the base case of inference — no unification): an int literal is `Int`
(default width; per-width/signedness refinement is a later slice, checked only at T4), a bool `Bool`, a
string `String`, a char `Char`. `none` if the node is not a scalar-literal atom. -/
def scalarLitTy? (m : Ast.Module) (nodeId : Nat) : Option Ty :=
  match m.nodes[nodeId]? with
  | some (.atom lid) =>
    match m.leaves[lid]? with
    | some (.intLit _ _ _) => some (.int 64 true)
    | some (.boolLit _) => some .bool
    | some (.str _) => some .string
    | some (.char _) => some .char
    | _ => none
  | _ => none

/-- Infer the type of a body node. T1.1a: scalar literals only (rule base case) — the extraction plumbing
all further HM rules build on. Non-literal bodies decline (`Unsupported`) until the V/A/Fn/App/Let/If/Match
rules land (design PROPOSAL-lean-type-oracle-typing-rules §3). -/
def inferBody (m : Ast.Module) (nodeId : Nat) : TypeVerdict :=
  match scalarLitTy? m nodeId with
  | some τ => .wellTyped τ
  | none => .unsupported "type oracle: non-literal body not yet modeled (T1.1a — HM rules land next)"

/-- The type oracle (T1.1a). Extract the `main` export's body (`Eval.namedParamsBody?`); a NULLARY `main`
whose body is a scalar literal is `WellTyped`. A parameterized main or a non-literal body declines
(`Unsupported` — a sound coverage gap). HM name-resolution + unification over fn/app/let/if/match lands in
the following T1 slices. -/
def infer (m : Ast.Module) : TypeVerdict :=
  match Eval.namedParamsBody? m "main".toUTF8 with
  | none => .unsupported "type oracle: program has no (def (main) …) export"
  | some (specs, bodyId) =>
    if specs.size != 0 then .unsupported "type oracle: parameterized main not yet modeled (T1)"
    else inferBody m bodyId

/-- The differential classification (design §1.2): map the oracle's verdict against rcdzc's carried
accept/reject/decline onto `holds`/`mismatch`/`skip`. A `mismatch` names the direction so cdz-smith triages
without re-deriving. `Unsupported` on the oracle side is ALWAYS a `skip` (a sound coverage gap — growing
coverage can only ADD checks, never create a false alarm). Code parity on an agreed reject is deferred to
T3 (both-reject ⇒ `holds` here, regardless of code). -/
def judgeTypecheck (tv : TypeVerdict) (rv : RcdzcVerdict) : Verdict :=
  match tv, rv with
  | .unsupported r, _ => .skip s!"typecheck: {r}"
  | .wellTyped _, .accept => .holds
  | .wellTyped _, .reject code =>
    .mismatch s!"false-reject: oracle infers well-typed, rcdzc rejected {code}"
  | .wellTyped _, .decline =>
    .mismatch s!"capability-gap: oracle infers well-typed, rcdzc declined (should-work-not-yet-built)"
  | .illTyped code, .accept =>
    .mismatch s!"false-accept: oracle infers ill-typed ({code}), rcdzc accepted (soundness hole)"
  | .illTyped _, .reject _ => .holds   -- both reject; code parity is a T3 refinement
  | .illTyped _, .decline => .holds    -- both reject/decline → agree

/-! ### Verdict-classification witnesses (compiled = checked; the §1.2 table). -/

-- an empty / main-less module → the oracle declines → skip (sound coverage gap)
#guard judgeTypecheck (infer { leaves := #[], nodes := #[], root := 0 }) .accept
       == .skip "typecheck: type oracle: program has no (def (main) …) export"
-- T1.1a: a nullary `main` whose body is an int literal is WellTyped Int (base case + main-body extraction).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                            .intLit false .dec (ByteArray.mk #[42]), .name "export".toUTF8],
                nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
                           .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
                root := 9 } == .wellTyped (.int 64 true))
-- a bool-literal main → WellTyped Bool; against an rcdzc ACCEPT this judges `holds` (agree).
#guard judgeTypecheck (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                                           .boolLit true, .name "export".toUTF8],
                               nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
                                          .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
                               root := 9 }) .accept == .holds
-- a non-literal body (an application) → the oracle declines (T1.1a) → skip.
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                                  .name "f".toUTF8, .name "export".toUTF8],
                      nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[3], .list #[0, 2, 4],
                                 .atom 4, .atom 2, .list #[6, 7], .atom 0, .list #[9, 5, 8]],
                      root := 10 } with | .unsupported _ => true | _ => false)
-- accept ∧ well-typed → agree
#guard judgeTypecheck (.wellTyped .bool) .accept == .holds
-- both reject (any code) → agree (T1); decline ∧ ill-typed → agree
#guard judgeTypecheck (.illTyped "CDZ0203") (.reject "CDZ0201") == .holds
#guard judgeTypecheck (.illTyped "CDZ0203") .decline == .holds
-- FALSE-REJECT — the highest-value finding: oracle accepts, rcdzc coded-rejected
#guard judgeTypecheck (.wellTyped .unit) (.reject "CDZ0203")
       == .mismatch "false-reject: oracle infers well-typed, rcdzc rejected CDZ0203"
-- CAPABILITY-GAP — oracle accepts, rcdzc codeless-declined (should-work-not-yet-built)
#guard judgeTypecheck (.wellTyped .unit) .decline
       == .mismatch "capability-gap: oracle infers well-typed, rcdzc declined (should-work-not-yet-built)"
-- FALSE-ACCEPT / soundness hole — oracle rejects, rcdzc accepted (reached once inference lands)
#guard (judgeTypecheck (.illTyped "CDZ0203") .accept != .holds)

end Oracle
