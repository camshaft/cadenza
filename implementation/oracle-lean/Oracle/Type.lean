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
  | never                               -- the empty sum; unifies with ANY type (ts:76-84, the bottom rule)
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

/-! ### The unification engine (the Algorithm-W workhorse, design §3 / PROPOSAL §3)

Pure, total-by-fuel-free-acyclicity type unification: a `Subst` maps unification variables to types, and
`unify` solves an equality constraint by extending it (or reports the CDZ code of the clash). This is the
CORE the A/App/If/Let/Fn rules all build on. It is deliberately landed and `#guard`-tested in ISOLATION
(no inference wires it yet), so it carries ZERO false-verdict risk while the rules that consume it land. -/

/-- Occurs check: does the unification variable `i` appear anywhere in `t`? A `var i` unified with a type
CONTAINING `i` is the infinite-type case — an unsatisfiable constraint (`CDZ0203`). -/
partial def occurs (i : Nat) : Ty → Bool
  | .var j => i == j
  | .fn d c => occurs i d || occurs i c
  | .tuple es => es.any (occurs i)
  | _ => false

/-- Does a type contain ANY (free) unification variable? Used by the App rule to decline applying a
POLYMORPHIC function type — a monomorphic oracle without `let`-generalization would otherwise false-reject
a polymorphic let-bound fn used at two types, so a var-containing head declines (`Unsupported`) instead. -/
partial def hasVar : Ty → Bool
  | .var _ => true
  | .fn d c => hasVar d || hasVar c
  | .tuple es => es.any hasVar
  | _ => false

/-- A unification substitution: variable id → resolved type, innermost (head) binding wins. -/
abbrev Subst := List (Nat × Ty)

/-- Resolve a type under a substitution, chasing variable chains to a fixpoint. Terminates because `unify`
occurs-checks before every binding, so `Subst` stays acyclic. -/
partial def applySubst (s : Subst) : Ty → Ty
  | .var i => match s.find? (fun p => p.1 == i) with
              | some (_, t) => applySubst s t
              | none => .var i
  | .fn d c => .fn (applySubst s d) (applySubst s c)
  | .tuple es => .tuple (es.map (applySubst s))
  | t => t

/-- Unify two types under `s` → the extended substitution, or the CDZ code of the clash. `never` (the empty
sum) unifies with ANYTHING (`ts:82`, the bottom rule — this is why `(if c 1 (trap …))` is well-typed at
`Int`); a var binds (occurs-checked); structural forms (`fn`/`tuple`) recurse; a head mismatch, width/sign
clash, or arity mismatch is a `CDZ0203` TypeMismatch (`ts:38`). -/
partial def unify (a b : Ty) (s : Subst) : Except Code Subst :=
  match applySubst s a, applySubst s b with
  | .never, _ => .ok s
  | _, .never => .ok s
  | .var i, .var j => if i == j then .ok s else .ok ((i, .var j) :: s)
  | .var i, t => if occurs i t then .error "CDZ0203" else .ok ((i, t) :: s)
  | t, .var i => if occurs i t then .error "CDZ0203" else .ok ((i, t) :: s)
  | .int w1 g1, .int w2 g2 => if w1 == w2 && g1 == g2 then .ok s else .error "CDZ0203"
  | .bool, .bool => .ok s
  | .unit, .unit => .ok s
  | .string, .string => .ok s
  | .char, .char => .ok s
  | .fn d1 c1, .fn d2 c2 => do let s ← unify d1 d2 s; unify c1 c2 s
  | .tuple e1, .tuple e2 =>
      if e1.length == e2.length then
        (e1.zip e2).foldlM (fun s (p : Ty × Ty) => unify p.1 p.2 s) s
      else .error "CDZ0203"
  | _, _ => .error "CDZ0203"

/-- Test helper: did unification fail with exactly `code`? (`Except` has no `BEq`, so match explicitly.) -/
def unifyIsErr (r : Except Code Subst) (code : Code) : Bool :=
  match r with | .error c => c == code | .ok _ => false

/-! ### Unification witnesses (compiled = checked). -/
-- like heads unify; a width/sign or head clash is CDZ0203
#guard (unify (.int 64 true) (.int 64 true) []).toOption.isSome
#guard unifyIsErr (unify (.int 64 true) (.int 32 true) []) "CDZ0203"
#guard unifyIsErr (unify .bool (.int 64 true) []) "CDZ0203"
-- a var binds, then resolves to its type under the returned subst
#guard (match unify (.var 0) .bool [] with | .ok s => applySubst s (.var 0) == .bool | _ => false)
-- occurs check: `var 0` in `(fn (var 0) bool)` → infinite type → CDZ0203
#guard unifyIsErr (unify (.var 0) (.fn (.var 0) .bool) []) "CDZ0203"
-- structural fn: domains + codomains unify pointwise
#guard (unify (.fn (.int 64 true) .bool) (.fn (.int 64 true) .bool) []).toOption.isSome
#guard unifyIsErr (unify (.fn (.int 64 true) .bool) (.fn .bool .bool) []) "CDZ0203"
-- structural tuple + arity
#guard (unify (.tuple [.int 64 true, .bool]) (.tuple [.int 64 true, .bool]) []).toOption.isSome
#guard unifyIsErr (unify (.tuple [.int 64 true]) (.tuple [.int 64 true, .bool]) []) "CDZ0203"
-- `never` is bottom: unifies with any type, either side
#guard (unify .never (.int 64 true) []).toOption.isSome
#guard (unify (.tuple [.int 64 true]) .never []).toOption.isSome
-- transitivity through the subst: bind `var 0 := Int`, then it unifies with Int but clashes with Bool
#guard (match unify (.var 0) (.int 64 true) [] with
        | .ok s => (unify (.var 0) (.int 64 true) s).toOption.isSome
                   && unifyIsErr (unify (.var 0) .bool s) "CDZ0203"
        | _ => false)

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

/-- Parse a TYPE-annotation node to a `Ty` over the modeled scalar subset: `Int64`/`(Int N)`/`UInt8`/… →
`.int N signed` (a `.bits` width; `BigInt`/`(Int W)` unknown-width → `none`, not modeled), `Bool`/`Unit`/
`String`/`Char` → their scalar `Ty`. `none` for any un-modeled annotation (records/sums/fn/generics) — the
ascription rule declines (`Unsupported`) on `none`, never guesses. Reused by Fn param annotations later. -/
def parseTy? (m : Ast.Module) (nodeId : Nat) : Option Ty :=
  match Eval.parseIntTy? m nodeId with
  | some it => (match it.width with | .bits n => some (.int n it.signed) | _ => none)
  | none =>
    match m.nodes[nodeId]? with
    | some (.atom lid) =>
      match m.leaves[lid]? with
      | some (.name b) =>
        (match String.fromUTF8? b with
         | some "Bool" => some .bool
         | some "Unit" => some .unit
         | some "String" => some .string
         | some "Char" => some .char
         | _ => none)
      | _ => none
    | _ => none

/-- An inference FAILURE: a positive `IllTyped` (a modeled fault with a CDZ code — a `mismatch` when it
disagrees with rcdzc) vs an `Unsupported` coverage gap (always a `skip`). Keeping them distinct is the
positive-disagreement invariant (design §5): the oracle emits a positive verdict ONLY on a fully-modeled
program, so an unresolved name / an unmodeled construct fails as `unsupported`, never as a false reject. -/
inductive InferFail where
  | illTyped (code : Code)
  | unsupported (reason : String)

/-- The threaded inference state: the accumulated unification substitution + the next fresh var id. -/
structure InferState where
  subst : Subst := []
  next : Nat := 0          -- next fresh unification-var id (used once App/Let/Fn introduce fresh vars)
  deriving Inhabited

/-- Lift `unify` into the inference result: a clash becomes a positive `IllTyped` (the code), success
threads the extended substitution. -/
def unifyInfer (a b : Ty) (st : InferState) : Except InferFail InferState :=
  match unify a b st.subst with
  | .ok s => .ok { st with subst := s }
  | .error c => .error (.illTyped c)

/-- Recursive HM inference over the analyzable T1 fragment: synthesize a type + threaded state, or fail
(`IllTyped code` / `Unsupported reason`).
* T1.1a — scalar literal → its type.
* T1.1b — the **V rule** (`ts:36`): a bare name resolves against `env` (top-level value defs); an
  UNRESOLVED name is `Unsupported` (NOT `CDZ0101` — sound without a full scope model, see `InferFail`).
* T1.2 — the **If rule** (`ts:76-84`): `(if c t e)` unifies `τc` with `Bool`, unifies the two branch
  types (`never` absorbs — `unify`'s bottom rule — so `(if c 1 (trap))` stays well-typed at `Int`), and
  yields the resolved branch type. A condition-not-`Bool` or a branch clash is `IllTyped CDZ0203`.
* T1.3 — **comparison / equality** (`< > <= >=` and `=`, `ts:186-188`): `(OP a b)` unifies the two
  operand types (a shape mismatch is a genuine type error) and yields `Bool`; an operand clash is `CDZ0203`.
* T1.4 — **arithmetic** (`+ - * / %`, §4): `(OP a b)` unifies the operands and requires the result to be
  numeric (`Int` → that int type); a same-typed non-numeric operand is `IllTyped CDZ0301`, a mixed clash
  `CDZ0203`. A Float operand (not a modeled scalar) → `Unsupported` (never a false `Int`-reject).
* T1.5 — **boolean connectives** (`and`/`or` binary, `not` unary): every operand unifies with `Bool`,
  result `Bool`; a non-`Bool` operand is `IllTyped CDZ0203`.
* T1.6 — **tuple construction** (`ts:130-146`): `(tuple e…)` infers each element → `.tuple [τ…]` (arity
  is part of the type); an `IllTyped`/`Unsupported` element propagates.
* T1.7 — **positional projection** (`ts:146`): `(. base i)` with an int index `i` yields the `i`-th
  element type of `base`'s tuple; out-of-arity or a non-tuple base is `IllTyped CDZ0203`. A name index
  (record field / member) is `Unsupported`.
* T1.8 — **let** (`ts:40-44`, monomorphic): `(let ((x e)…) body)` binds each `x:τ` sequentially (later
  sees earlier) then infers `body`; complete for the fn-free fragment (generalization lands with Fn).
* T1.9 — **do block**: `(do stmt… last)` — a value def `(def x e)` binds `x:τ` sequentially, a non-def
  statement is inferred (well-typed, type discarded), and the `last` item is the result. A local function
  def `(def (f …) …)` is `Unsupported` (needs Fn).
* T1.10 — **ascription** (`(: e T)`, `ts:50-54`), constrain-not-override: unify `τ` with `T`, result `T`;
  a category clash is `CDZ0203`; an unmodeled `T`, or an int↔int width ascription (deferred to OQ-G), is
  `Unsupported` (never a false width-reject).
* T1.11 — **Fn** (`ts:28-36`): `(fn (p…) body)` gives each param a fresh var (bare) or its annotated
  type, infers `body`, and yields the curried arrow `p₁→…→body`.
* T1.12 — **App** (`ts:36`): `(f a…)` with `f` a name bound to a CONCRETE function type unifies the
  arrow against each arg to a fresh result var → the codomain; a domain clash / non-fn head is `CDZ0203`.
  A polymorphic (var-containing) head or an unbound head → `Unsupported` (defers `let`-generalization).
Any other construct → `Unsupported` until its rule lands (Match). -/
partial def inferE (m : Ast.Module) (env : List (ByteArray × Ty)) (st : InferState) (nodeId : Nat) :
    Except InferFail (Ty × InferState) :=
  match scalarLitTy? m nodeId with
  | some τ => .ok (τ, st)
  | none =>
    match Eval.nameOf? m nodeId with
    | some nm =>
      match env.find? (fun e => e.1 == nm) with
      | some (_, τ) => .ok (τ, st)
      | none => .error (.unsupported
          "type oracle: unresolved name (may be a prelude/builtin or local binder — CDZ0101 unbound needs the prelude scope model)")
    | none =>
      match m.nodes[nodeId]? with
      | some (.list children) =>
        match m.headName? (.list children) with
        | some h =>
          if h == "if".toUTF8 then
            match children[1]?, children[2]?, children[3]? with
            | some cId, some tId, some eId => do
                let (τc, st) ← inferE m env st cId
                let st ← unifyInfer τc .bool st          -- condition must be Bool (ts:76)
                let (τt, st) ← inferE m env st tId
                let (τe, st) ← inferE m env st eId
                let st ← unifyInfer τt τe st              -- both branches unify; never absorbs (ts:82-84)
                .ok (applySubst st.subst τt, st)
            | _, _, _ => .error (.unsupported "type oracle: malformed if")
          else if (String.fromUTF8? h).elim false (fun s => Eval.cmpOps.contains s || s == "=") then
            -- T1.3 — COMPARISON / EQUALITY (`< > <= >=` and `=`): `(OP a b)` unifies the two operand types
            -- (a shape mismatch is a genuine type error — ts:186-188) and yields `Bool`. An operand clash is
            -- `IllTyped CDZ0203`. SOUND for the current fragment: only scalars flow to a comparison (the
            -- Fn/tuple rules that could produce a non-orderable operand aren't wired yet); the orderable-vs-
            -- equatable distinction on COMPOUND operands is a refinement for when those become inferable.
            match children[1]?, children[2]? with
            | some aId, some bId => do
                let (τa, st) ← inferE m env st aId
                let (τb, st) ← inferE m env st bId
                let st ← unifyInfer τa τb st
                .ok (.bool, st)
            | _, _ => .error (.unsupported "type oracle: malformed comparison")
          else if (String.fromUTF8? h).elim false (fun s => Eval.arithOps.contains s) then
            -- T1.4 — ARITHMETIC (`+ - * / %`): `(OP a b)` unifies the two operands, then requires the result
            -- to be NUMERIC — `Int` → result that int type; a same-typed NON-numeric operand (`Bool`/`String`/
            -- `Char`/`Unit`) is `IllTyped CDZ0301` NumericMismatch (§4). A mixed operand clash was already
            -- caught by the unify (`CDZ0203`). `never` absorbs (`(+ (trap) x)`). SOUND on float: a float
            -- literal isn't a modeled scalar (`scalarLitTy?` declines it) → its operand is `Unsupported` →
            -- the whole expr skips, never a false `Int`-reject of valid Float arithmetic. A still-unresolved
            -- operand type → `Unsupported` (can't classify numeric-ness).
            match children[1]?, children[2]? with
            | some aId, some bId => do
                let (τa, st) ← inferE m env st aId
                let (τb, st) ← inferE m env st bId
                let st ← unifyInfer τa τb st
                match applySubst st.subst τa with
                | .int w sg => .ok (.int w sg, st)
                | .never => .ok (.never, st)
                | .bool | .string | .char | .unit => .error (.illTyped "CDZ0301")
                | _ => .error (.unsupported "type oracle: arithmetic on an unresolved/unmodeled operand type")
            | _, _ => .error (.unsupported "type oracle: malformed arithmetic (unary or partial)")
          else if (String.fromUTF8? h).elim false (fun s => s == "and" || s == "or" || s == "not") then
            -- T1.5 — BOOLEAN connectives (`and`/`or` binary, `not` unary): every operand unifies with
            -- `Bool`, result `Bool`. A non-`Bool` operand is `IllTyped CDZ0203`.
            if h == "not".toUTF8 then
              match children[1]? with
              | some aId => do
                  let (τa, st) ← inferE m env st aId
                  let st ← unifyInfer τa .bool st
                  .ok (.bool, st)
              | none => .error (.unsupported "type oracle: malformed not")
            else
              match children[1]?, children[2]? with
              | some aId, some bId => do
                  let (τa, st) ← inferE m env st aId
                  let st ← unifyInfer τa .bool st
                  let (τb, st) ← inferE m env st bId
                  let st ← unifyInfer τb .bool st
                  .ok (.bool, st)
              | _, _ => .error (.unsupported "type oracle: malformed and/or")
          else if h == "tuple".toUTF8 then do
            -- T1.6 — TUPLE construction (`ts:130-146`): `(tuple e…)` infers each element and yields
            -- `.tuple [τ…]` (arity is part of the type). A tuple is well-typed iff EVERY element is — an
            -- element that is `IllTyped`/`Unsupported` propagates (short-circuits the fold).
            let elemIds := children.extract 1 children.size
            let (τs, st) ← elemIds.foldlM (fun (acc : List Ty × InferState) eid => do
                let (τ, st') ← inferE m env acc.2 eid
                pure (acc.1 ++ [τ], st')) ([], st)
            .ok (.tuple τs, st)
          else if h == ".".toUTF8 && children.size == 3 then
            -- T1.7 — positional TUPLE PROJECTION `(. base i)` where `i` is an INT literal (`ts:146`): infer
            -- `base` as a tuple, then the `i`-th element type if `i < arity`, else out-of-arity is
            -- `IllTyped CDZ0203`; a positional projection of a NON-tuple is also a type error (`CDZ0203`).
            -- A NAME index (record-field / module-member access) is `Unsupported` until records/members are
            -- modeled — a name index never decodes to an int (`Value.ofLeaf`), so it falls through cleanly.
            match children[1]?, children[2]? with
            | some baseId, some idxId =>
              match (m.nodes[idxId]?).bind (fun n => match n with | .atom lid => m.leaves[lid]? | _ => none) with
              | some l =>
                (match Value.ofLeaf l with
                 | some (.int n) =>
                   if n < 0 then .error (.illTyped "CDZ0203")
                   else do
                     let (τb, st) ← inferE m env st baseId
                     match applySubst st.subst τb with
                     | .tuple τs => (match τs[n.toNat]? with
                                     | some τ => .ok (τ, st)
                                     | none => .error (.illTyped "CDZ0203"))
                     | .never => .ok (.never, st)
                     | _ => .error (.illTyped "CDZ0203")
                 | _ => .error (.unsupported
                     "type oracle: record-field / member projection not yet modeled"))
              | none => .error (.unsupported "type oracle: malformed projection")
            | _, _ => .error (.unsupported "type oracle: malformed projection")
          else if h == "let".toUTF8 then
            -- T1.8 — LET (`ts:40-44`), MONOMORPHIC: `(let ((x e)…) body)` infers each binding value and
            -- extends the env with `x:τ` SEQUENTIALLY (a later binding sees the earlier), then infers the
            -- body under the extended env. Monomorphic (no ∀-generalization) — COMPLETE for the current
            -- fn-free fragment (a let-bound value has no polymorphic reuse without lambdas); generalization
            -- lands with the Fn rule. An `IllTyped`/`Unsupported` binding value propagates.
            match children[1]?, children[2]? with
            | some bindingsId, some bodyId =>
              (match m.nodes[bindingsId]? with
               | some (Ast.Node.list pairs) =>
                 (match pairs.foldlM (m := Except InferFail) (fun (acc : List (ByteArray × Ty) × InferState) pid =>
                     match m.nodes[pid]? with
                     | some (Ast.Node.list pc) =>
                       (match pc[0]?, pc[1]? with
                        | some nId, some vId =>
                          (match Eval.nameOf? m nId with
                           | some nm =>
                             (match inferE m acc.1 acc.2 vId with
                              | .ok (τ, st') => .ok (((nm, τ) :: acc.1), st')
                              | .error e => .error e)
                           | none => .error (.unsupported "type oracle: let binding missing name"))
                        | _, _ => .error (.unsupported "type oracle: malformed let binding"))
                     | _ => .error (.unsupported "type oracle: let binding not a (name value) pair"))
                     (env, st) with
                  | .ok (env', st') => inferE m env' st' bodyId
                  | .error e => .error e)
               | _ => .error (.unsupported "type oracle: let bindings not a list"))
            | _, _ => .error (.unsupported "type oracle: malformed let")
          else if h == "do".toUTF8 then
            -- T1.9 — DO block (`(do stmt… last)`): process each leading statement, then the LAST item is
            -- the result. A value def `(def x e)` binds `x:τ` into the env SEQUENTIALLY (mirrors `let`); a
            -- non-def statement is inferred (must be well-typed — an ill-typed statement makes the whole
            -- program ill-typed) and its type discarded. A LOCAL FUNCTION def `(def (f …) …)` needs the Fn
            -- rule → `Unsupported` until that lands. An `IllTyped`/`Unsupported` statement propagates.
            let items := children.extract 1 children.size
            match items.back? with
            | none => .error (.unsupported "type oracle: empty do")
            | some lastId =>
              let stmts := items.extract 0 (items.size - 1)
              (match stmts.foldlM (m := Except InferFail) (fun (acc : List (ByteArray × Ty) × InferState) sid =>
                  match Eval.asDef? m sid with
                  | some dc =>
                    (match dc[1]?, dc[dc.size - 1]? with
                     | some targetId, some valId =>
                       (match Eval.nameOf? m targetId with
                        | some nm =>                       -- value def → bind x:τ
                          (match inferE m acc.1 acc.2 valId with
                           | .ok (τ, st') => .ok (((nm, τ) :: acc.1), st')
                           | .error e => .error e)
                        | none => .error (.unsupported "type oracle: local function def in a do not yet modeled"))
                     | _, _ => .error (.unsupported "type oracle: malformed do def"))
                  | none =>                                -- non-def statement → must be well-typed, type discarded
                    (match inferE m acc.1 acc.2 sid with
                     | .ok (_, st') => .ok (acc.1, st')
                     | .error e => .error e))
                  (env, st) with
               | .ok (env', st') => inferE m env' st' lastId
               | .error e => .error e)
          else if h == ":".toUTF8 && children.size == 3 then
            -- T1.10 — ASCRIPTION `(: e T)` (`ts:50-54`), constrain-NOT-override: parse `T`, infer `e`,
            -- unify `τ` with `T`; the result is `T`. A clash across type CATEGORIES (`(: 5 Bool)`,
            -- `(: #t Int64)`, …) is `IllTyped CDZ0203`. An unmodeled annotation type → `Unsupported`.
            -- 🪤 INT↔INT ascription (`(: 5 Int32)`) is DEFERRED to the OQ-G fresh-width-var model →
            -- `Unsupported`, NOT a reject: my scalar int literal is concrete `.int 64`, so a strict
            -- width-unify would FALSE-REJECT a width-polymorphic literal. (Category clashes are still caught.)
            match children[1]?, children[2]? with
            | some eId, some tId =>
              (match parseTy? m tId with
               | none => .error (.unsupported "type oracle: unmodeled ascription type")
               | some τT => do
                   let (τ, st) ← inferE m env st eId
                   match applySubst st.subst τ, τT with
                   | .int _ _, .int _ _ => .error (.unsupported "type oracle: int-width ascription deferred (OQ-G)")
                   | τr, _ =>
                     (match unifyInfer τr τT st with
                      | .ok st' => .ok (τT, st')
                      | .error e => .error e))
            | _, _ => .error (.unsupported "type oracle: malformed ascription")
          else if h == "fn".toUTF8 then
            -- T1.11 — Fn / closure introduction (`ts:28-36`): `(fn (p…) body)` gives each param a FRESH type
            -- var (a bare `x`) or its annotated type (`(: x T)` via `parseTy?`), infers `body` under the
            -- extended env, and yields the CURRIED arrow `p₁→…→body` (`inferBody`'s final `applySubst`
            -- resolves any param var the body constrained). An annotated param whose type isn't modeled →
            -- `Unsupported`. This is the intro rule; App (elim) reads the arrow back in a follow-up slice.
            match children[1]?, children[2]? with
            | some paramsId, some bodyId =>
              (match m.nodes[paramsId]? with
               | some (Ast.Node.list paramNodes) =>
                 (match paramNodes.foldlM (m := Except InferFail)
                     (fun (acc : List (ByteArray × Ty) × List Ty × InferState) pid =>
                       match m.nodes[pid]? with
                       | some (Ast.Node.atom lid) =>            -- bare param → fresh var
                         (match m.leaves[lid]? with
                          | some (.name nm) =>
                            let α : Ty := .var acc.2.2.next
                            .ok ((nm, α) :: acc.1, α :: acc.2.1, { acc.2.2 with next := acc.2.2.next + 1 })
                          | _ => .error (.unsupported "type oracle: malformed fn param"))
                       | some (Ast.Node.list pc) =>            -- (: name T)
                         (match pc[1]?, pc[2]? with
                          | some nId, some tId =>
                            (match Eval.nameOf? m nId, parseTy? m tId with
                             | some nm, some τ => .ok ((nm, τ) :: acc.1, τ :: acc.2.1, acc.2.2)
                             | some _, none => .error (.unsupported "type oracle: fn param has an unmodeled type annotation")
                             | none, _ => .error (.unsupported "type oracle: fn param missing name"))
                          | _, _ => .error (.unsupported "type oracle: malformed fn param spec"))
                       | none => .error (.unsupported "type oracle: malformed fn param"))
                     (env, [], st) with
                  | .ok (env', ptysRev, st') =>
                    (match inferE m env' st' bodyId with
                     | .ok (bodyτ, st'') => .ok (ptysRev.foldl (fun acc pτ => Ty.fn pτ acc) bodyτ, st'')
                     | .error e => .error e)
                  | .error e => .error e)
               | _ => .error (.unsupported "type oracle: fn params not a list"))
            | _, _ => .error (.unsupported "type oracle: malformed fn")
          else
            -- T1.12 — APPLICATION `(f a…)` (`ts:36`), the arrow-elim rule: `f` a NAME bound in the env to a
            -- function; unify the (curried) fn type against each argument to a fresh result var, yielding the
            -- codomain. 🪤 SOUNDNESS: only apply a CONCRETE (no-free-var) function type — a POLYMORPHIC head
            -- (e.g. a let-bound `(fn (x) x) : α→α`) declines to `Unsupported`, because monomorphic
            -- instantiation without `let`-generalization would FALSE-REJECT it used at two types (design §5).
            -- A head not bound in the env (a prelude/builtin) → `Unsupported`. `(f)` (no args) = grouping →
            -- `f`'s type. Applying a non-function concrete type, or an arg-domain clash → `IllTyped CDZ0203`.
            match env.find? (fun e => e.1 == h) with
            | some (_, τf0) =>
              let τf := applySubst st.subst τf0
              if hasVar τf then
                .error (.unsupported "type oracle: polymorphic application not modeled (needs let-generalization)")
              else
                (match (children.extract 1 children.size).foldlM (m := Except InferFail)
                    (fun (acc : Ty × InferState) aid =>
                      match inferE m env acc.2 aid with
                      | .ok (τa, st1) =>
                        let β : Ty := .var st1.next
                        let st2 := { st1 with next := st1.next + 1 }
                        (match unifyInfer acc.1 (.fn τa β) st2 with
                         | .ok st3 => .ok (applySubst st3.subst β, st3)
                         | .error e => .error e)
                      | .error e => .error e) (τf, st) with
                 | .ok (τres, st') => .ok (τres, st')
                 | .error e => .error e)
            | none => .error (.unsupported
                "type oracle: unmodeled application head / construct (App/Match — prelude & polymorphic heads decline)")
        | none => .error (.unsupported "type oracle: non-name-headed construct not yet modeled")
      | _ => .error (.unsupported "type oracle: node not modeled")

/-- Infer the type of a body node under a value environment `env`: run `inferE` and map its result onto
the verdict algebra. A resolved type (with the final substitution applied) is `WellTyped`; a modeled
fault is `IllTyped`; a coverage gap is `Unsupported`. -/
def inferBody (m : Ast.Module) (env : List (ByteArray × Ty)) (nodeId : Nat) : TypeVerdict :=
  match inferE m env {} nodeId with
  | .ok (τ, st) => .wellTyped (applySubst st.subst τ)
  | .error (.illTyped c) => .illTyped c
  | .error (.unsupported r) => .unsupported r

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
-- T1.2 (If rule): `(if #t 1 2)` — both branches Int, condition Bool → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "if".toUTF8,
                            .boolLit true, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .list #[0, 1, 2, 3],  -- (if #t 1 2)
                           .atom 2, .list #[5], .atom 1, .list #[7, 6, 4],           -- (def (main) …)
                           .atom 7, .atom 2, .list #[9, 10], .atom 0, .list #[12, 8, 11]],
                root := 13 } == .wellTyped (.int 64 true))
-- T1.2 (If rule): `(if #t 1 #f)` — branch types clash (Int vs Bool) → IllTyped CDZ0203 (the FIRST
-- positive reject the oracle emits; against an rcdzc ACCEPT this is a false-accept mismatch).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "if".toUTF8,
                            .boolLit true, .intLit false .dec (ByteArray.mk #[1]), .boolLit false,
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .list #[0, 1, 2, 3],
                           .atom 2, .list #[5], .atom 1, .list #[7, 6, 4],
                           .atom 7, .atom 2, .list #[9, 10], .atom 0, .list #[12, 8, 11]],
                root := 13 } == .illTyped "CDZ0203")
-- T1.2 (If rule): `(if 1 2 3)` — condition is Int, not Bool → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "if".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .list #[0, 1, 2, 3],
                           .atom 2, .list #[5], .atom 1, .list #[7, 6, 4],
                           .atom 7, .atom 2, .list #[9, 10], .atom 0, .list #[12, 8, 11]],
                root := 13 } == .illTyped "CDZ0203")
-- T1.3 (comparison): `(< 1 2)` — operands unify (Int, Int), result Bool → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "<".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],       -- (< 1 2)
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],    -- (def (main) …)
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped .bool)
-- T1.3 (comparison): `(< 1 #t)` — operand clash (Int vs Bool) → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "<".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0203")
-- T1.3 × T1.2 integration: `(if (< 1 2) 10 20)` — comparison condition is Bool → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "if".toUTF8,
                            .name "<".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .intLit false .dec (ByteArray.mk #[10]),
                            .intLit false .dec (ByteArray.mk #[20]), .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],           -- (< 1 2)
                           .atom 3, .atom 7, .atom 8, .list #[4, 3, 5, 6],        -- (if (< 1 2) 10 20)
                           .atom 2, .list #[8], .atom 1, .list #[10, 9, 7],       -- (def (main) …)
                           .atom 9, .atom 2, .list #[12, 13], .atom 0, .list #[15, 11, 14]],
                root := 16 } == .wellTyped (.int 64 true))
-- T1.4 (arithmetic): `(+ 1 2)` — numeric operands → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "+".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped (.int 64 true))
-- T1.4 (arithmetic): `(+ #t #f)` — same-typed but NON-numeric operands → IllTyped CDZ0301 NumericMismatch.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "+".toUTF8,
                            .boolLit true, .boolLit false, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0301")
-- T1.4 (arithmetic): `(+ 1 #t)` — mixed operand clash → IllTyped CDZ0203 (caught by unify before the numeric check).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "+".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0203")
-- T1.5 (boolean): `(and #t #f)` — both operands Bool → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "and".toUTF8,
                            .boolLit true, .boolLit false, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped .bool)
-- T1.5 (boolean): `(not #t)` — unary; operand Bool → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "not".toUTF8,
                            .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .list #[0, 1],            -- (not #t)
                           .atom 2, .list #[3], .atom 1, .list #[5, 4, 2],
                           .atom 5, .atom 2, .list #[7, 8], .atom 0, .list #[10, 6, 9]],
                root := 11 } == .wellTyped .bool)
-- T1.5 (boolean): `(and #t 1)` — a non-Bool operand → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "and".toUTF8,
                            .boolLit true, .intLit false .dec (ByteArray.mk #[1]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0203")
-- T1.6 (tuple): `(tuple 1 #t)` → WellTyped (tuple [Int, Bool]) — arity + element types.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "tuple".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped (.tuple [.int 64 true, .bool]))
-- T1.6 (tuple): `(tuple 1 (if #t 2 #f))` — the ill-typed element (branch clash) PROPAGATES → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "tuple".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .name "if".toUTF8, .boolLit true,
                            .intLit false .dec (ByteArray.mk #[2]), .boolLit false, .name "export".toUTF8],
                nodes := #[.atom 5, .atom 6, .atom 7, .atom 8, .list #[0, 1, 2, 3],  -- (if #t 2 #f)
                           .atom 3, .atom 4, .list #[5, 6, 4],                       -- (tuple 1 (if …))
                           .atom 2, .list #[8], .atom 1, .list #[10, 9, 7],          -- (def (main) …)
                           .atom 9, .atom 2, .list #[12, 13], .atom 0, .list #[15, 11, 14]],
                root := 16 } == .illTyped "CDZ0203")
-- T1.7 (projection): `(. (tuple 1 #t) 1)` → the element-1 type = Bool → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "tuple".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .boolLit true,
                            .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (tuple 1 #t)
                           .atom 3, .atom 5, .list #[4, 3, 5],            -- (. (tuple 1 #t) 1)
                           .atom 2, .list #[7], .atom 1, .list #[9, 8, 6],-- (def (main) …)
                           .atom 7, .atom 2, .list #[11, 12], .atom 0, .list #[14, 10, 13]],
                root := 15 } == .wellTyped .bool)
-- T1.7 (projection): `(. (tuple 1 #t) 5)` — index out of arity → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "tuple".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .boolLit true,
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (tuple 1 #t)
                           .atom 3, .atom 7, .list #[4, 3, 5],            -- (. (tuple 1 #t) 5)
                           .atom 2, .list #[7], .atom 1, .list #[9, 8, 6],
                           .atom 8, .atom 2, .list #[11, 12], .atom 0, .list #[14, 10, 13]],
                root := 15 } == .illTyped "CDZ0203")
-- T1.8 (let): `(let ((x 5)) x)` — x bound to Int, body resolves it → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                            .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .list #[0, 1],       -- (x 5)
                           .list #[2],                            -- ((x 5))
                           .atom 4,                               -- body x
                           .atom 3, .list #[5, 3, 4],             -- (let ((x 5)) x)
                           .atom 2, .list #[7], .atom 1, .list #[9, 8, 6],  -- (def (main) …)
                           .atom 6, .atom 2, .list #[11, 12], .atom 0, .list #[14, 10, 13]],
                root := 15 } == .wellTyped (.int 64 true))
-- T1.8 (let): `(let ((b #t)) (if b 1 2))` — a let-bound Bool used as an if condition → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                            .name "b".toUTF8, .boolLit true, .name "if".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .list #[0, 1],       -- (b #t)
                           .list #[2],                            -- ((b #t))
                           .atom 6, .atom 4, .atom 7, .atom 8, .list #[4, 5, 6, 7],  -- (if b 1 2)
                           .atom 3, .list #[9, 3, 8],             -- (let ((b #t)) (if b 1 2))
                           .atom 2, .list #[11], .atom 1, .list #[13, 12, 10],       -- (def (main) …)
                           .atom 9, .atom 2, .list #[15, 16], .atom 0, .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.int 64 true))
-- T1.9 (do): main body `(do (def x 5) (+ x 1))` — a local value def then an arith body → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "x".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "+".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .name "export".toUTF8],
                nodes := #[.atom 1, .atom 3, .atom 4, .list #[0, 1, 2],   -- (def x 5)
                           .atom 5, .atom 3, .atom 6, .list #[4, 5, 6],   -- (+ x 1)
                           .atom 0, .list #[8, 3, 7],                     -- (do (def x 5) (+ x 1))
                           .atom 2, .list #[10], .atom 1, .list #[12, 11, 9],  -- (def (main) <inner do>)
                           .atom 7, .atom 2, .list #[14, 15], .atom 0, .list #[17, 13, 16]],
                root := 18 } == .wellTyped (.int 64 true))
-- T1.9 (do): `(do (def b #t) (if b 1 2))` — a do-bound Bool used as an if condition → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "b".toUTF8,
                            .boolLit true, .name "if".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 1, .atom 3, .atom 4, .list #[0, 1, 2],   -- (def b #t)
                           .atom 5, .atom 3, .atom 6, .atom 7, .list #[4, 5, 6, 7],  -- (if b 1 2)
                           .atom 0, .list #[9, 3, 8],                     -- (do (def b #t) (if b 1 2))
                           .atom 2, .list #[11], .atom 1, .list #[13, 12, 10],  -- (def (main) <inner do>)
                           .atom 8, .atom 2, .list #[15, 16], .atom 0, .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.int 64 true))
-- T1.10 (ascription): `(: #t Bool)` — matching category → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .boolLit true, .name "Bool".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped .bool)
-- T1.10 (ascription): `(: 5 Bool)` — category clash (Int vs Bool) → IllTyped CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "Bool".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0203")
-- T1.10 (ascription): `(: 5 Int64)` — int↔int width ascription is DEFERRED (OQ-G) → Unsupported, NOT a reject.
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                                  .intLit false .dec (ByteArray.mk #[5]), .name "Int64".toUTF8, .name "export".toUTF8],
                      nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                                 .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                                 .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                      root := 12 } with | .unsupported _ => true | _ => false)
-- T1.11 (Fn): `(fn (x) (+ x 1))` — the arithmetic body constrains the fresh param var to Int → Int→Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "fn".toUTF8,
                            .name "x".toUTF8, .name "+".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "export".toUTF8],
                nodes := #[.atom 4, .list #[0],                   -- (x)
                           .atom 5, .atom 4, .atom 6, .list #[2, 3, 4],  -- (+ x 1)
                           .atom 3, .list #[6, 1, 5],             -- (fn (x) (+ x 1))
                           .atom 2, .list #[8], .atom 1, .list #[10, 9, 7],  -- (def (main) <fn>)
                           .atom 7, .atom 2, .list #[12, 13], .atom 0, .list #[15, 11, 14]],
                root := 16 } == .wellTyped (.fn (.int 64 true) (.int 64 true)))
-- T1.11 (Fn): `(fn ((: b Bool)) (if b 1 2))` — an annotated param + an if body → Bool→Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "fn".toUTF8,
                            .name ":".toUTF8, .name "b".toUTF8, .name "Bool".toUTF8, .name "if".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (: b Bool)
                           .list #[3],                            -- ((: b Bool))
                           .atom 7, .atom 5, .atom 8, .atom 9, .list #[5, 6, 7, 8],  -- (if b 1 2)
                           .atom 3, .list #[10, 4, 9],            -- (fn ((: b Bool)) (if b 1 2))
                           .atom 2, .list #[12], .atom 1, .list #[14, 13, 11],  -- (def (main) <fn>)
                           .atom 10, .atom 2, .list #[16, 17], .atom 0, .list #[19, 15, 18]],
                root := 20 } == .wellTyped (.fn .bool (.int 64 true)))
-- T1.12 (App): `(let ((f (fn (x) (+ x 1)))) (f 5))` — a concrete Int→Int fn applied to Int → WellTyped Int.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                            .name "f".toUTF8, .name "fn".toUTF8, .name "x".toUTF8, .name "+".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[5]),
                            .name "export".toUTF8],
                nodes := #[.atom 7, .atom 6, .atom 8, .list #[0, 1, 2],   -- (+ x 1)
                           .atom 6, .list #[4],                           -- (x)
                           .atom 5, .list #[6, 5, 3],                     -- (fn (x) (+ x 1))
                           .atom 4, .list #[8, 7],                        -- (f (fn …))
                           .list #[9],                                    -- ((f (fn …)))
                           .atom 4, .atom 9, .list #[11, 12],             -- (f 5)
                           .atom 3, .list #[14, 10, 13],                  -- (let ((f …)) (f 5))
                           .atom 2, .list #[16], .atom 1, .list #[18, 17, 15],  -- (def (main) <let>)
                           .atom 10, .atom 2, .list #[20, 21], .atom 0, .list #[23, 19, 22]],
                root := 24 } == .wellTyped (.int 64 true))
-- T1.12 (App): `(let ((f (fn (x) (+ x 1)))) (f #t))` — Int→Int applied to Bool → arg clash → CDZ0203.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                            .name "f".toUTF8, .name "fn".toUTF8, .name "x".toUTF8, .name "+".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 7, .atom 6, .atom 8, .list #[0, 1, 2],
                           .atom 6, .list #[4], .atom 5, .list #[6, 5, 3],
                           .atom 4, .list #[8, 7], .list #[9],
                           .atom 4, .atom 9, .list #[11, 12],
                           .atom 3, .list #[14, 10, 13],
                           .atom 2, .list #[16], .atom 1, .list #[18, 17, 15],
                           .atom 10, .atom 2, .list #[20, 21], .atom 0, .list #[23, 19, 22]],
                root := 24 } == .illTyped "CDZ0203")
-- T1.12 (App): `(let ((id (fn (x) x))) (id 5))` — a POLYMORPHIC head (id : α→α) declines → Unsupported
-- (NOT a false reject: monomorphic instantiation without let-generalization would be unsound).
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                                  .name "id".toUTF8, .name "fn".toUTF8, .name "x".toUTF8,
                                  .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                      nodes := #[.atom 6, .atom 6, .list #[1], .atom 5, .list #[3, 2, 0],  -- x, (x), (fn (x) x)
                                 .atom 4, .list #[5, 4], .list #[6],       -- (id (fn …)), ((id (fn …)))
                                 .atom 4, .atom 7, .list #[8, 9],          -- (id 5)
                                 .atom 3, .list #[11, 7, 10],              -- (let ((id …)) (id 5))
                                 .atom 2, .list #[13], .atom 1, .list #[15, 14, 12],
                                 .atom 8, .atom 2, .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                      root := 21 } with | .unsupported _ => true | _ => false)
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
