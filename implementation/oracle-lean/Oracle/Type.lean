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

/-- The type of a top-level VALUE definition `(def x <scalar-literal>)`: the target is a bare NAME atom
(a value binding — a `(def (f …) …)` function def has a `(name …)`-LIST target, so `Eval.nameOf?` returns
`none` on it and this excludes it) and the body is a scalar literal. `none` for a function def, a
`(name …)`-list target, or a non-literal body. This is the T1.1b value environment: enough to type a
`main` body that ALIASES a top-level literal binding. Extended to full HM-typed defs as fn/let/app land. -/
def topLevelValueDefTy? (m : Ast.Module) (defChildren : Array Nat) : Option (ByteArray × Ty) := do
  let tid ← defChildren[1]?
  let nm ← Eval.nameOf? m tid              -- bare-name target ⇒ a value def (list target ⇒ function def ⇒ none)
  let bodyId ← defChildren[defChildren.size - 1]?
  let τ ← scalarLitTy? m bodyId
  some (nm, τ)

/-- The T1.1b top-level value environment: `(name, τ)` for every top-level `(def x <scalar-literal>)` in
the `(do …)` program. The V rule (design §3, `ts:36`) resolves a body name against it. -/
def topLevelValueEnv (m : Ast.Module) : List (ByteArray × Ty) :=
  match m.nodes[m.root]? with
  | some (.list stmts) =>
    stmts.toList.filterMap (fun sid =>
      match Eval.asDef? m sid with
      | some dc => topLevelValueDefTy? m dc
      | none => none)
  | _ => []

/-- Infer the type of a body node under a value environment `env`.
* T1.1a: a scalar literal → its type (the rule base case).
* T1.1b — the **V rule** (design §3 row V, `ts:36`): a bare-name body resolves against `env` (top-level
  value defs); a resolved name gets its bound type. An UNRESOLVED name declines (`Unsupported`) rather
  than emitting `CDZ0101 Unbound` — a positive unbound-reject is UNSOUND without a complete scope model
  (the name may be a prelude/builtin or a not-yet-modeled local binder), so CDZ0101 waits on the prelude
  scope slice. This keeps the positive-disagreement invariant (design §5): the oracle only ever emits a
  positive verdict on a fully-modeled program.
Any other non-literal body declines until the A/App/If/Let/Fn/Match rules land. -/
def inferBody (m : Ast.Module) (env : List (ByteArray × Ty)) (nodeId : Nat) : TypeVerdict :=
  match scalarLitTy? m nodeId with
  | some τ => .wellTyped τ
  | none =>
    match Eval.nameOf? m nodeId with
    | some nm =>
      match env.find? (fun e => e.1 == nm) with
      | some (_, τ) => .wellTyped τ
      | none => .unsupported
          "type oracle: unresolved name (may be a prelude/builtin or local binder — CDZ0101 unbound needs the prelude scope model)"
    | none => .unsupported "type oracle: non-literal body not yet modeled (T1.1b — HM app/if/let/fn/match rules land next)"

/-- The type oracle (T1.1b). Extract the `main` export's body (`Eval.namedParamsBody?`), build the
top-level value environment, and infer the body: a scalar literal (T1.1a) or a name resolving to a
top-level value binding (T1.1b, the V rule) is `WellTyped`. A parameterized main, an unresolved name, or
any other non-literal body declines (`Unsupported` — a sound coverage gap). HM unification over
app/if/let/fn/match lands in the following T1 slices. -/
def infer (m : Ast.Module) : TypeVerdict :=
  match Eval.namedParamsBody? m "main".toUTF8 with
  | none => .unsupported "type oracle: program has no (def (main) …) export"
  | some (specs, bodyId) =>
    if specs.size != 0 then .unsupported "type oracle: parameterized main not yet modeled (T1)"
    else inferBody m (topLevelValueEnv m) bodyId

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
-- T1.1b (V rule): `(do (def x 5) (def (main) x) (export main))` — main's body ALIASES the top-level
-- value def `x`, which resolves to Int → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "x".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "main".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2],       -- (def x 5)
                           .atom 1, .atom 4, .list #[5], .atom 2, .list #[4, 6, 7],  -- (def (main) x)
                           .atom 5, .atom 4, .list #[9, 10],                  -- (export main)
                           .atom 0, .list #[12, 3, 8, 11]],                   -- (do …)
                root := 13 } == .wellTyped (.int 64 true))
-- T1.1b: an UNRESOLVED name body → declines (`Unsupported`, NOT a CDZ0101 positive reject — the name
-- may be a prelude/builtin) → against an rcdzc reject this judges `skip`, never a false-reject.
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                                  .name "foo".toUTF8, .name "export".toUTF8],
                      nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],  -- (def (main) foo)
                                 .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
                      root := 9 } with | .unsupported _ => true | _ => false)
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
