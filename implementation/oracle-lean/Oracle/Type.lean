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
  | bool | unit | string | char | bytes   -- `bytes` (T1.39): a param-less leaf like `string` (a heap byte-sequence)
  | rational                               -- `rational` (T1.44): a param-less leaf — exact rational, totally ordered
  | bigint                                 -- `bigint` (T1.46): a param-less leaf — arbitrary-precision integer
  | fn (dom cod : Ty)                   -- curried function
  | tuple (elts : List Ty)
  | record (fields : List (ByteArray × Ty))  -- CLOSED record, fields sorted by key + unique (ts:70-74)
  | sum (variants : List (ByteArray × Option Ty))  -- CLOSED sum, variants sorted by name; `none` payload =
                                        -- nullary variant (ts:192-204). Option/Result/Ordering + user sums.
  | listTy (elem : Ty)                  -- a homogeneous `List` of `elem` (T1.29 — collection type modeling)
  | setTy (elem : Ty)                   -- a homogeneous `Set` of `elem` (T1.32 — set collection type, mirrors listTy)
  | mapTy (key val : Ty)                -- a `Map` from `key` to `val` (T1.33 — map collection type, two params)
  | never                               -- the empty sum; unifies with ANY type (ts:76-84, the bottom rule)
  | var (id : Nat)                      -- a unification (type) variable
  | numVar (id : Nat)                   -- a NUMERIC unification variable (an unannotated int literal): unifies
                                        -- with any `int` width (OQ-G width-polymorphism) but NOT with a
                                        -- non-numeric type — so `(< #t 1)` still clashes, while `(: 5 Int32)` /
                                        -- `(+ (x:Int32) 1)` resolve the literal to the annotated/param width.
  | float (bits : Nat)                  -- a fixed-width float (Float32 / Float64) — T1.40
  | floatVar (id : Nat)                 -- a FLOAT unification variable (an unannotated float literal): the
                                        -- float twin of `numVar` — unifies with any `float` width (default
                                        -- Float64), NOT with a non-float, so `(+ (x:Float32) 1.5)` resolves
                                        -- the literal to Float32 and mixed int/float still clashes (CDZ0203).
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
  | .numVar j => i == j
  | .floatVar j => i == j
  | .fn d c => occurs i d || occurs i c
  | .tuple es => es.any (occurs i)
  | .listTy e => occurs i e
  | .setTy e => occurs i e
  | .mapTy k v => occurs i k || occurs i v
  | .record fs => fs.any (fun f => occurs i f.2)
  | .sum vs => vs.any (fun v => match v.2 with | some t => occurs i t | none => false)
  | _ => false

/-- Does a type contain ANY (free) unification variable? Used by the App rule to decline applying a
POLYMORPHIC function type — a monomorphic oracle without `let`-generalization would otherwise false-reject
a polymorphic let-bound fn used at two types, so a var-containing head declines (`Unsupported`) instead. -/
partial def hasVar : Ty → Bool
  | .var _ => true
  | .numVar _ => true
  | .floatVar _ => true
  | .fn d c => hasVar d || hasVar c
  | .tuple es => es.any hasVar
  | .listTy e => hasVar e
  | .setTy e => hasVar e
  | .mapTy k v => hasVar k || hasVar v
  | .record fs => fs.any (fun f => hasVar f.2)
  | .sum vs => vs.any (fun v => match v.2 with | some t => hasVar t | none => false)
  | _ => false

/-- Does a type contain a GENERAL (non-numeric) unification var? The App rule declines a head with one of
these (true `let`-polymorphism, e.g. `(fn (x) x) : α→α`). A `numVar`-only head (e.g. `numVar→Int` from
`(fn (x) (+ x 1))`) is NOT truly polymorphic — a `numVar` only unifies with numeric types and defaults to
`Int` — so applying it is sound (an arg clash still surfaces as `CDZ0203`). -/
partial def hasGenVar : Ty → Bool
  | .var _ => true
  | .fn d c => hasGenVar d || hasGenVar c
  | .tuple es => es.any hasGenVar
  | .listTy e => hasGenVar e
  | .setTy e => hasGenVar e
  | .mapTy k v => hasGenVar k || hasGenVar v
  | .record fs => fs.any (fun f => hasGenVar f.2)
  | .sum vs => vs.any (fun v => match v.2 with | some t => hasGenVar t | none => false)
  | _ => false

/-- Does the type contain a `.setTy` or `.mapTy` anywhere (recursively through compounds)? A set/map value
carries NO blessed total order (19-sets:4340-4351, the all-leaf sibling of the float case), so `Set.to-list`
over a set whose ELEMENT contains a set/map is a coded CDZ0203 — ordered enumeration is undefined. (Set
CONSTRUCTION / contains / insert / union still work; only the ORDER-based to-list declines.) -/
partial def containsSetOrMap : Ty → Bool
  | .setTy _ => true
  | .mapTy _ _ => true
  | .float _ => true       -- a FLOAT carries no blessed total order either (19-sets:4340 — set/map are the "all-leaf sibling of the FLOAT case")
  | .floatVar _ => true    -- an undetermined float literal defaults to Float64, also non-orderable
  | .listTy e => containsSetOrMap e
  | .tuple es => es.any containsSetOrMap
  | .fn d c => containsSetOrMap d || containsSetOrMap c
  | .record fs => fs.any (fun f => containsSetOrMap f.2)
  | .sum vs => vs.any (fun v => match v.2 with | some t => containsSetOrMap t | none => false)
  | _ => false

/-- NEWTYPE ERASURE (05-compound-types:8519): a SINGLE-variant sum carrying ONE payload (`(type T (Mk τ))`)
is a newtype — at runtime the value IS its payload `τ` (the tag erases). So member/field access sees THROUGH
the tag to the payload. Unwrap such single-variant-single-payload sums repeatedly (a newtype over a newtype
… over a record). A multi-variant sum, or a single NULLARY variant, is NOT erased (a real sum / unit-like). -/
partial def eraseNewtypes : Ty → Ty
  | .sum [(_, some τ)] => eraseNewtypes τ
  | t => t

/-- The `#symbol` (`.sym` leaf) bytes a node references, if it is an `atom → sym` leaf. Used for record
row-op field keys `(Record.with r #k v)` — `#k` is a symbol, not a name (T1.48). -/
def symOf? (m : Ast.Module) (nid : Nat) : Option ByteArray :=
  match m.nodes[nid]? with
  | some (.atom lid) => (match m.leaves[lid]? with | some (.sym b) => some b | _ => none)
  | _ => none

/-- Read a Record row op's LITERAL field-name LIST operand `(a c)` — a `.list` node whose children are each
a bare `.name` leaf (a LABEL, not an evaluated value; `project`/`without`'s 2nd operand). `none` if the node
is not a list or any child is not a bare name (a malformed label list → the oracle declines rather than
guessing rcdzc's CDZ0201). -/
def labelBytesOf? (m : Ast.Module) (k : Nat) : Option ByteArray :=
  match m.nodes[k]? with
  | some (.atom lid) => (match m.leaves[lid]? with | some (.name b) => some b | _ => none)
  | _ => none

/-- Collect a label list's bare-name children (structural recursion — `#guard`-evaluable, unlike `mapM`). -/
def collectLabels? (m : Ast.Module) : List Nat → Option (List ByteArray)
  | []      => some []
  | k :: ks =>
    match labelBytesOf? m k, collectLabels? m ks with
    | some b, some rest => some (b :: rest)
    | _, _              => none

def labelsOf? (m : Ast.Module) (nid : Nat) : Option (List ByteArray) :=
  match m.nodes[nid]? with
  | some (.list kids) => collectLabels? m kids.toList
  | _ => none

/-- Does a label list contain a DUPLICATE (by ByteArray value)? A record's fields are a fixed SET of names,
so a label named twice in a `project`/`without` list is CDZ0201 (matches a duplicate record-literal field). -/
def hasDupBytes : List ByteArray → Bool
  | []      => false
  | x :: xs => xs.any (fun y => Eval.cmpBytes x y == .eq) || hasDupBytes xs

/-- Parse a width-namespaced INTEGER MODULE name (`Int64`/`UInt8`/…) → `(width, signed)`. Only the realized
fixed widths 8/16/32/64 (bare `Int`/`UInt`/`BigInt` → `none`, no fixed width). Used by the int-module ops
`(. Int64 max)` / `(UInt8.wrap …)` etc. (T1.43). -/
def intWidthName? (q : ByteArray) : Option (Nat × Bool) :=
  match String.fromUTF8? q with
  | some "Int8"  => some (8, true)   | some "Int16"  => some (16, true)
  | some "Int32" => some (32, true)  | some "Int64"  => some (64, true)
  | some "UInt8"  => some (8, false)  | some "UInt16" => some (16, false)
  | some "UInt32" => some (32, false) | some "UInt64" => some (64, false)
  | _ => none

/-- A unification substitution: variable id → resolved type, innermost (head) binding wins. -/
abbrev Subst := List (Nat × Ty)

/-- Resolve a type under a substitution, chasing variable chains to a fixpoint. Terminates because `unify`
occurs-checks before every binding, so `Subst` stays acyclic. -/
partial def applySubst (s : Subst) : Ty → Ty
  | .var i => match s.find? (fun p => p.1 == i) with
              | some (_, t) => applySubst s t
              | none => .var i
  | .numVar i => match s.find? (fun p => p.1 == i) with
                 | some (_, t) => applySubst s t
                 | none => .numVar i
  | .floatVar i => match s.find? (fun p => p.1 == i) with
                   | some (_, t) => applySubst s t
                   | none => .floatVar i
  | .fn d c => .fn (applySubst s d) (applySubst s c)
  | .tuple es => .tuple (es.map (applySubst s))
  | .listTy e => .listTy (applySubst s e)
  | .setTy e => .setTy (applySubst s e)
  | .mapTy k v => .mapTy (applySubst s k) (applySubst s v)
  | .record fs => .record (fs.map (fun f => (f.1, applySubst s f.2)))
  | .sum vs => .sum (vs.map (fun v => (v.1, v.2.map (applySubst s))))
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
  -- a NUMERIC var (an int literal, OQ-G): unifies with another numVar or with a concrete int (resolving to
  -- that width); NOT with a non-numeric type (falls to the `_,_` clash — so `(< #t 1)` is still CDZ0203).
  | .numVar i, .numVar j => if i == j then .ok s else .ok ((i, .numVar j) :: s)
  | .numVar i, .int w g => .ok ((i, .int w g) :: s)
  | .int w g, .numVar i => .ok ((i, .int w g) :: s)
  | .int w1 g1, .int w2 g2 => if w1 == w2 && g1 == g2 then .ok s else .error "CDZ0203"
  -- a FLOAT var (a float literal): the float twin of the numVar arms — unifies with another floatVar or a
  -- concrete float width; a float-width clash (Float32 vs Float64) is CDZ0203. Does NOT unify with .int/.numVar
  -- (mixed int/float falls to the `_,_` clash → CDZ0203, matching rcdzc's no-implicit-int↔float rule).
  | .floatVar i, .floatVar j => if i == j then .ok s else .ok ((i, .floatVar j) :: s)
  | .floatVar i, .float w => .ok ((i, .float w) :: s)
  | .float w, .floatVar i => .ok ((i, .float w) :: s)
  | .float w1, .float w2 => if w1 == w2 then .ok s else .error "CDZ0203"
  -- an int LITERAL grounds to BigInt (BigInt is an INTEGER type, so a `.numVar` unifies with it exactly as
  -- with a fixed width — 06-numeric-model:1233-1235: a bare literal adopts its bigint peer / an i64-
  -- overflowing literal grounds to bigint; and `Int64.of`/`UInt8.of` narrowing a bigint source works because
  -- their fresh-numVar arg absorbs it). This is UNLIKE Rational (a distinct numeric kind → ascription-only).
  | .numVar i, .bigint => .ok ((i, .bigint) :: s)
  | .bigint, .numVar i => .ok ((i, .bigint) :: s)
  | .bool, .bool => .ok s
  | .unit, .unit => .ok s
  | .string, .string => .ok s
  | .char, .char => .ok s
  | .bytes, .bytes => .ok s
  | .rational, .rational => .ok s
  | .bigint, .bigint => .ok s
  | .fn d1 c1, .fn d2 c2 => do let s ← unify d1 d2 s; unify c1 c2 s
  | .tuple e1, .tuple e2 =>
      if e1.length == e2.length then
        (e1.zip e2).foldlM (fun s (p : Ty × Ty) => unify p.1 p.2 s) s
      else .error "CDZ0203"
  | .listTy e1, .listTy e2 => unify e1 e2 s        -- Lists unify iff their element types unify
  | .setTy e1, .setTy e2 => unify e1 e2 s          -- Sets unify iff their element types unify
  | .mapTy k1 v1, .mapTy k2 v2 => do let s ← unify k1 k2 s; unify v1 v2 s  -- Maps: keys then values
  | .record f1, .record f2 =>
      -- CLOSED records unify iff same field-name SET (stored sorted+unique, so zip-compare keys) and each
      -- field's type unifies (ts:70-74, ts:184-190). A field-set or field-type mismatch is CDZ0203.
      if f1.length == f2.length
         && (f1.zip f2).all (fun (p : (ByteArray × Ty) × (ByteArray × Ty)) => p.1.1 == p.2.1) then
        (f1.zip f2).foldlM (fun s (p : (ByteArray × Ty) × (ByteArray × Ty)) => unify p.1.2 p.2.2 s) s
      else .error "CDZ0203"
  | .sum v1, .sum v2 =>
      -- CLOSED sums unify iff same variant-name SET (stored sorted) AND each payload option matches (both
      -- nullary, or both carry a payload whose types unify) — ts:192-204. Otherwise CDZ0203.
      if v1.length == v2.length
         && (v1.zip v2).all (fun (p : (ByteArray × Option Ty) × (ByteArray × Option Ty)) => p.1.1 == p.2.1) then
        (v1.zip v2).foldlM (fun s (p : (ByteArray × Option Ty) × (ByteArray × Option Ty)) =>
          match p.1.2, p.2.2 with
          | none, none => .ok s
          | some t1, some t2 => unify t1 t2 s
          | _, _ => .error "CDZ0203") s
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
    -- a FLOAT literal (`3.5` / `nan` / `inf`) — returns `.float 64` as the "it's a float literal" MARKER;
    -- `inferE` intercepts it (like the int marker) and allocates a fresh width-poly `.floatVar` instead.
    | some (.float _ _ _) => some (.float 64)
    | some .floatNan => some (.float 64)
    | some (.floatInf _) => some (.float 64)
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
def topLevelValueEnv (m : Ast.Module) : List (ByteArray × (List Nat × Ty)) :=
  match m.nodes[m.root]? with
  | some (.list stmts) =>
    stmts.toList.filterMap (fun sid =>
      match Eval.asDef? m sid with
      | some dc => (topLevelValueDefTy? m dc).map (fun p => (p.1, ([], p.2)))   -- top-level scalar def → mono scheme
      | none => none)
  | _ => []

/-- WHOLE-PROGRAM soundness gate (design §5): `infer` may only emit a POSITIVE verdict for a program it
FULLY models. Since `infer` types only `main`'s body, it must DECLINE (`Unsupported`) any program whose
OTHER top-level structure it does not vet — else it over-claims `WellTyped` on a program whose other
defs/exports/declarations are ill-typed (the false-reject class the T0.2 corpus run surfaced: unbound
names in a non-main def, user generics, `(type …)` declarations, unbound exports). Conservative: the root
`(do …)` must contain ONLY {a SCALAR value def `(def x <lit>)`, the `main` def, an `(export nm)` naming a
DEFINED name, a `(pragma …)`}. A fn-def, a non-scalar value def, a `(type …)`/`(effect …)`/import, or an
export of an unbound name ⇒ not fully modeled ⇒ decline. (Typing fn-defs / non-scalar value defs is a
later increment; declining them here is SOUND — a `skip`, never a false reject.) -/
def programModeled? (m : Ast.Module) : Bool :=
  match m.nodes[m.root]? with
  | some (.list stmtsRaw) =>
    let stmts := stmtsRaw.extract 1 stmtsRaw.size    -- drop the `do` head atom; children[1:] are the statements
    let definedNames : List ByteArray :=
      stmts.toList.filterMap (fun sid => (Eval.asDef? m sid).bind (Eval.defName? m))
    stmts.all (fun sid =>
      match Eval.asDef? m sid with
      | some dc =>
        -- T1.24: any def (bare-name value def OR list-target fn-def) is a MODELED KIND — `topLevelEnv`
        -- types it (an ill-typed/unmodeled def → IllTyped/Unsupported there). Only a malformed def (no
        -- target) is rejected here; `(type …)`/`(effect …)`/import stmts fall to the `none` arm below.
        (dc[1]?).isSome
      | none =>
        match m.nodes[sid]? with
        | some node =>
          (match m.headName? node with
           | some h =>
             if h == "export".toUTF8 then
               -- EVERY exported name must resolve to a definition (a multi-name `(export a b …)` scans ALL
               -- names — an undefined 2nd+ name is rcdzc CDZ0101; checking only the first was a false-reject).
               (match node with
                | .list cs => (cs.extract 1 cs.size).all (fun nid =>
                                match Eval.nameOf? m nid with
                                | some nm => definedNames.any (· == nm)
                                | none => false)
                | _ => false)
             else if h == "type".toUTF8 then true    -- T1.25: a `(type …)` decl is a modeled KIND; its
                                                      -- sound modelability is gated by `userSumMap` in `infer`
             else h == "pragma".toUTF8
           | none => false)
        | none => false)
  | _ => false

/-- The built-in sum types (variants sorted by name, per the `.sum` canonical form). -/
def optionTy (τ : Ty) : Ty := .sum [("None".toUTF8, none), ("Some".toUTF8, some τ)]
def resultTy (ok err : Ty) : Ty := .sum [("Err".toUTF8, some err), ("Ok".toUTF8, some ok)]
def orderingTy : Ty := .sum [("Equal".toUTF8, none), ("Greater".toUTF8, none), ("Less".toUTF8, none)]

/-- Parse a TYPE-annotation node to a `Ty` over the modeled subset: `Int64`/`(Int N)`/`UInt8`/… → `.int N
signed` (a `.bits` width; `BigInt`/`(Int W)` unknown-width → `none`), `Bool`/`Unit`/`String`/`Char` → their
scalar `Ty`, `Ordering` → the Ordering sum, the built-in sum CONSTRUCTORS `(Option T)` / `(Result T E)`
→ their `.sum`, and a FUNCTION type `(-> t1 … tn)` → the curried arrow `t1→…→tn` (all parsed recursively).
`none` for any un-modeled annotation (records / user sums / generics, or a compound with an unmodeled
part) — the ascription/param rule declines (`Unsupported`) on `none`, never guesses. -/
partial def parseTy? (m : Ast.Module) (nodeId : Nat) : Option Ty :=
  match m.nodes[nodeId]? with
  | some (.list cs) =>
    (match m.headName? (.list cs) with
     | some h =>
       -- reject a MALFORMED width-indexed constructor `(Int w1 w2)` — `Int`/`UInt` take exactly ONE width
       -- arg (`Eval.parseIntTy?` is lenient, so guard arity here → `none`, the sound decline).
       if h == "Int".toUTF8 || h == "UInt".toUTF8 then
         (if cs.size != 2 then none
          else match Eval.parseIntTy? m nodeId with
               | some it => (match it.width with | .bits n => some (.int n it.signed) | _ => none)
               | none => none)
       else if h == "Option".toUTF8 && cs.size == 2 then
         (match cs[1]? with | some p => (parseTy? m p).map optionTy | none => none)
       else if h == "Result".toUTF8 && cs.size == 3 then
         (match cs[1]?, cs[2]? with
          | some okId, some errId =>
            (match parseTy? m okId, parseTy? m errId with
             | some okT, some errT => some (resultTy okT errT)
             | _, _ => none)
          | _, _ => none)
       else if h == "List".toUTF8 && cs.size == 2 then
         (match cs[1]? with | some p => (parseTy? m p).map Ty.listTy | none => none)
       else if h == "Set".toUTF8 && cs.size == 2 then
         (match cs[1]? with | some p => (parseTy? m p).map Ty.setTy | none => none)
       else if h == "Map".toUTF8 && cs.size == 3 then
         (match cs[1]?, cs[2]? with
          | some kId, some vId =>
            (match parseTy? m kId, parseTy? m vId with
             | some kT, some vT => some (.mapTy kT vT)
             | _, _ => none)
          | _, _ => none)
       else if h == "Tuple".toUTF8 && cs.size >= 3 then
         -- tuple TYPE constructor `(Tuple T1 T2 … Tn)` (n ≥ 2) → `.tuple [T1…Tn]`. (`Tuple` is the type
         -- ctor; `tuple` is the value ctor.) Each element parsed recursively; an unmodeled element → decline.
         (match (cs.extract 1 cs.size).toList.mapM (parseTy? m) with
          | some ts => some (.tuple ts)
          | none => none)
       else if h == "Record".toUTF8 && cs.size >= 2 then
         -- record TYPE constructor `(Record (: k1 T1) (: k2 T2) …)` — each field is `(: name Type)` (size-3,
         -- head `:`) or the bare `(name Type)` (size-2) → `.record` (fields sorted by key, the canonical
         -- form so `unify` compares field SETs). A non-name key, an unmodeled field type, or a DUPLICATE
         -- field name → decline (`none`). A field whose type is a nominal/recursive user type → its
         -- `parseTy?` is `none` → the whole record declines (sound; never a false-accept).
         (match (cs.extract 1 cs.size).toList.mapM (fun fid => do
             let fc ← (match m.nodes[fid]? with | some (.list fc) => some fc | _ => none)
             let kt ← (if m.headName? (.list fc) == some ":".toUTF8 && fc.size == 3 then some (fc[1]!, fc[2]!)
                       else if fc.size == 2 then some (fc[0]!, fc[1]!)
                       else none)
             let k ← Eval.nameOf? m kt.1
             let τ ← parseTy? m kt.2
             some (k, τ)) with
          | some fields =>
            let sorted := (fields.toArray.qsort (fun a b => Eval.cmpBytes a.1 b.1 == .lt)).toList
            if (sorted.zip (sorted.drop 1)).any (fun (p : (ByteArray × Ty) × (ByteArray × Ty)) => p.1.1 == p.2.1)
            then none                       -- duplicate field name → malformed record type
            else some (.record sorted)
          | none => none)
       else if h == "->".toUTF8 && cs.size >= 3 then
         -- function type `(-> t1 t2 … tn)` = `t1 → t2 → … → tn` (curried; last element = result). Each
         -- element parsed recursively; an unmodeled element → `none` (decline).
         (match (cs.extract 1 cs.size).toList.mapM (parseTy? m) with
          | some ts => (match ts.reverse with
                        | result :: revArgs => some (revArgs.foldl (fun acc t => Ty.fn t acc) result)
                        | [] => none)
          | none => none)
       else none
     | none => none)
  | some (.atom _) =>
    (match Eval.parseIntTy? m nodeId with
     | some it => (match it.width with | .bits n => some (.int n it.signed) | _ => none)
     | none =>
       (match m.nodes[nodeId]? with
        | some (.atom lid) =>
          (match m.leaves[lid]? with
           | some (.name b) =>
             (match String.fromUTF8? b with
              | some "Bool" => some .bool
              | some "Unit" => some .unit
              | some "String" => some .string
              | some "Char" => some .char
              | some "Bytes" => some .bytes
              | some "Float64" => some (.float 64)
              | some "Float32" => some (.float 32)
              | some "Rational" => some .rational
              | some "BigInt" => some .bigint
              | some "Ordering" => some orderingTy
              | _ => none)
           | _ => none)
        | _ => none))
  | _ => none

/-- A nullary `Ordering` constructor (`Less`/`Equal`/`Greater`) → the Ordering sum (var-free, so no fresh
var needed). `None` is handled inline (its Option payload needs a FRESH var). -/
def orderingCtorTy? (nm : ByteArray) : Option Ty :=
  match String.fromUTF8? nm with
  | some "Less" => some orderingTy
  | some "Equal" => some orderingTy
  | some "Greater" => some orderingTy
  | _ => none

/-- Is `vid` a `(doc …)` node (a variant-list doc annotation, not a variant)? -/
def isDocNode (m : Ast.Module) (vid : Nat) : Bool :=
  match m.nodes[vid]? with
  | some (Ast.Node.list vc) => m.headName? (Ast.Node.list vc) == some "doc".toUTF8
  | _ => false

/-- Parse ONE variant spec of a `(type …)` decl → `(ctorName, payload Ty option)`: a bare `A` or `(A)` is
NULLARY (`none`); `(A τ)` carries payload `parseTy? τ`. `none` (⇒ the whole type is UNMODELED) for a
payload that isn't modeled, or a variant arity > 1 (a variant is uniformly single-payload). -/
def userVariantSpec? (m : Ast.Module) (vid : Nat) : Option (ByteArray × Option Ty) :=
  match m.nodes[vid]? with
  | some (Ast.Node.atom lid) =>
    (match m.leaves[lid]? with | some (Ast.Leaf.name b) => some (b, none) | _ => none)
  | some (Ast.Node.list vc) =>
    (match m.headName? (Ast.Node.list vc) with
     | some cn => if vc.size == 1 then some (cn, none)
                  else if vc.size == 2 then (parseTy? m (vc[1]!)).map (fun τ => (cn, some τ))
                  else none
     | none => none)
  | none => none

/-- T1.25 — parse all top-level `(type T (A τ) (B) …)` decls into a VARIANT MAP: each variant name → (its
type's structural `.sum` Ty, its payload Ty option). `none` ⇒ the program is DECLINED (sound skip) because
the user sums aren't soundly modelable structurally: an unmodeled/compound payload or arity > 1 (a whole
type fails), OR a NOMINAL AMBIGUITY — a variant name shared across user types or colliding with a built-in
(Some/None/Ok/Err/Less/Equal/Greater). With globally-UNIQUE variant names, structural == nominal, so a
`.sum` faithfully models the nominal type. Variants kept in declaration order (consistent → self-unifies;
cross-type unify is impossible under the uniqueness gate). -/
def userSumMap (m : Ast.Module) : Option (List (ByteArray × (ByteArray × Ty × Option Ty))) :=
  match m.nodes[m.root]? with
  | some (Ast.Node.list stmts) =>
    (do
      let entries ← (stmts.extract 1 stmts.size).toList.foldlM (m := Option)
        (fun (acc : List (ByteArray × (ByteArray × Ty × Option Ty))) sid =>
          match m.nodes[sid]? with
          | some (Ast.Node.list tc) =>
            if m.headName? (Ast.Node.list tc) == some "type".toUTF8 then
              (do
                let tn := ((tc[1]?).bind (Eval.nameOf? m)).getD ByteArray.empty     -- the type's name (for qualified `T.A`)
                let vspecs ← ((tc.extract 2 tc.size).toList.filter (fun vid => !isDocNode m vid)).mapM (userVariantSpec? m)
                let sumTy : Ty := .sum vspecs
                pure (acc ++ vspecs.map (fun (cn, p) => (cn, (tn, sumTy, p)))))
            else pure acc
          | _ => pure acc) []
      -- collect the user TYPE names too (a user type may SHADOW a prelude type — e.g. `(type Int64 (A))` —
      -- which changes what `Int64` means in later annotations; our `parseTy?` uses the prelude meaning, so
      -- we must DECLINE such a shadow rather than false-accept).
      let typeNames := (stmts.extract 1 stmts.size).toList.filterMap (fun sid =>
        match m.nodes[sid]? with
        | some (Ast.Node.list tc) =>
          if m.headName? (Ast.Node.list tc) == some "type".toUTF8 then (tc[1]?).bind (Eval.nameOf? m) else none
        | _ => none)
      let names := entries.map (·.1)
      -- RESERVED names (built-in variants + built-in sum types + modeled scalar type names): a user variant
      -- OR type name colliding with any of these is a nominal shadow our structural model can't track soundly.
      let reserved : List ByteArray :=
        ["Some", "None", "Ok", "Err", "Less", "Equal", "Greater", "Option", "Result", "Ordering",
         "Int", "UInt", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64", "BigInt",
         "Bool", "Unit", "String", "Char", "Float32", "Float64"].map (·.toUTF8)
      if names.eraseDups.length == names.length             -- no duplicate variant name (nominal ambiguity)
         && typeNames.eraseDups.length == typeNames.length  -- no duplicate type name (two same-named types)
         && names.all (fun n => !reserved.contains n)       -- no variant shadows a built-in/type name
         && typeNames.all (fun n => !reserved.contains n)   -- no type name shadows a prelude/built-in type
         && (Eval.defNames m).all (fun dn => !reserved.contains dn)  -- no top-level def shadows a prelude
                                                                     -- TYPE name (else `parseTy?` on a payload
                                                                     -- annotation uses the wrong prelude meaning)
      then some entries else none)
  | _ => none

/-- Classify a MATCH pattern `patId` against the scrutinee sum's variant list `vs` (Mat rule, T1.16).
Returns `some (covered, catchAll, binds)` for a MODELED pattern: `covered` = the variant name(s) this
pattern matches (for the exhaustiveness check), `catchAll` = the pattern matches EVERY remaining value
(a `_` wildcard or a fresh binder), `binds` = payload binders introduced (name → type) to extend the
body env. Returns `none` for an UNMODELED pattern (a literal, a `(tuple …)`, a NESTED constructor
sub-pattern, or a foreign/user constructor) → the caller DECLINES the whole match (`Unsupported`),
never a false reject. Narrow first cut: only `_`, a bare binder, and the uniform `(Ctor binder)` form
with ONE flat binder/wildcard — `(Some x)`/`(Ok _)` (payload-bearing, binder REQUIRED) and `(None _)`/
`(Less _)` (nullary, unit payload) plus the bare `(None)` sugar (size-1, no binder). A bare name that IS
a variant of `vs` is that variant's nullary pattern (spec prelude-and-resolution §A), not a binder. -/
def matchPatClassify? (m : Ast.Module) (vs : List (ByteArray × Option Ty)) (scrutTy : Ty) (patId : Nat) :
    Option (List ByteArray × Bool × List (ByteArray × Ty)) :=
  let variantPayload? : ByteArray → Option (Option Ty) := fun nm => (vs.find? (·.1 == nm)).map (·.2)
  match m.nodes[patId]? with
  | some (.atom lid) =>
    (match m.leaves[lid]? with
     | some (.name b) =>
       if b == "_".toUTF8 then some ([], true, [])
       else match variantPayload? b with
            | some none => some ([b], false, [])              -- bare nullary variant pattern (None/Less/…)
            | some (some _) => none                           -- bare unary ctor name = ill-formed pattern → decline
            | none => some ([], true, [(b, scrutTy)])         -- fresh binder = catch-all, binds the whole value
     | _ => none)                                             -- a literal pattern on a sum → decline
  | some (.list pc) =>
    (match m.headName? (.list pc) with
     | some hp =>
       if hp == ".".toUTF8 then
         -- T1.28 — QUALIFIED nullary pattern `(. Q M)` (= `Q.M`): `M` must be a NULLARY variant of the
         -- scrutinee sum (`M ∈ vs`, payload none) AND `Q` its declaring type name (`userSumMap`). Covers {M}.
         (match (pc[1]?).bind (Eval.nameOf? m), (pc[2]?).bind (Eval.nameOf? m) with
          | some q, some mem =>
            (match variantPayload? mem, (userSumMap m).bind (fun mp => (mp.find? (fun e => e.1 == mem)).map (·.2.1)) with
             | some none, some tn => if tn == q then some ([mem], false, []) else none
             | _, _ => none)
          | _, _ => none)
       else
       -- A modeled variant pattern. Its payload type is `.unit` for a NULLARY variant (None/Less/…), else
       -- the variant's payload `τp` (Some/Ok/Err). Patterns are UNIFORMLY `(Ctor binder)` (spec
       -- core-semantics.md): `(Some x)`, `(None _)`, `(Ok _)`. A nullary variant ALSO admits the bare
       -- `(None)` (size 1) sugar with no binder; a payload-bearing variant REQUIRES its one binder.
       (match variantPayload? hp with
        | some payloadOpt =>
          let payloadTy : Ty := match payloadOpt with | some τp => τp | none => .unit
          if pc.size == 1 then
            (match payloadOpt with
             | none => some ([hp], false, [])                  -- `(None)` bare nullary (no binder)
             | some _ => none)                                 -- bare `(Some)` = ill-formed pattern → decline
          else if pc.size == 2 then
            (match pc[1]? with
             | some subId =>
               (match (m.nodes[subId]? : Option Ast.Node) with
                | some (Ast.Node.atom sl) =>
                  (match (m.leaves[sl]? : Option Ast.Leaf) with
                   | some (Ast.Leaf.name sb) =>
                     if sb == "_".toUTF8 then some ([hp], false, [])
                     else if (variantPayload? sb).isSome then none    -- a nested variant name → decline
                     else some ([hp], false, [(sb, payloadTy)])       -- flat binder for the payload
                   | _ => none)                                       -- a literal sub-pattern → decline
                | _ => none)                                          -- a nested ctor/tuple sub-pattern → decline
             | none => none)
          else none                                             -- over-applied pattern → decline
        | none => none)                                        -- a foreign / user constructor pattern → decline
     | none =>
       -- T1.28 — QUALIFIED payload pattern `((. Q M) binder)` (= `(Q.M binder)`): the head is `(. Q M)`
       -- (a list, so `headName?` is none). `M` must be a variant of the scrutinee sum and `Q` its declaring
       -- type; one flat binder/wildcard binds the payload. (Nullary `(. Q M)` with no binder is the case above.)
       (match Eval.qualHead? m pc with
        | some (q, mem) =>
          (match variantPayload? mem, (userSumMap m).bind (fun mp => (mp.find? (fun e => e.1 == mem)).map (·.2.1)) with
           | some payloadOpt, some tn =>
             if tn != q || pc.size != 2 then none
             else (match pc[1]? with
                   | some subId =>
                     (match (m.nodes[subId]? : Option Ast.Node) with
                      | some (Ast.Node.atom sl) =>
                        (match (m.leaves[sl]? : Option Ast.Leaf) with
                         | some (Ast.Leaf.name sb) =>
                           if sb == "_".toUTF8 then some ([mem], false, [])
                           else if (variantPayload? sb).isSome then none
                           else some ([mem], false, [(sb, match payloadOpt with | some τp => τp | none => .unit)])
                         | _ => none)
                      | _ => none)
                   | none => none)
           | _, _ => none)
        | none => none))
  | none => none

/-- Recursively bind an IRREFUTABLE sub-pattern `patId` against its expected type `τ` (T1.52 inc-4 —
NESTED sub-patterns). Returns `some binds` for a `_`/bare binder, OR a nested `(tuple p1…pn)` (vs a tuple
type, recursing each element) / `(record (= k p)…)` (vs a record type, exact keys, recursing each field) —
all IRREFUTABLE (tuple/record have a single shape). Returns `none` (→ the caller DECLINES) for a REFUTABLE
or unmodeled sub-pattern: a literal, a variant `(Ctor p)`, a nested list pattern, an arity/key/head/type
mismatch. Dedup of the collected binders is left to the top-level classifier. This is the shared engine for
the collection-pattern classifiers, so nesting composes (`#tuple(a #record((= x b)))`, etc.). -/
partial def subPatBinds? (m : Ast.Module) (τ : Ty) (patId : Nat) : Option (Bool × List (ByteArray × Ty)) :=
  match m.nodes[patId]? with
  | some (.atom lid) =>
    (match m.leaves[lid]? with
     | some (.name b) => some (true, if b == "_".toUTF8 then [] else [(b, τ)])  -- binder/`_`: irrefutable
     | _ => none)                                             -- a bare literal → refutable, no binds; defer (decline)
  | some (.list pc) =>
    (match m.headName? (.list pc), τ with
     | some hp, .tuple τs =>
       if hp == "tuple".toUTF8 then
         let sps := (pc.extract 1 pc.size).toList
         if sps.length == τs.length then
           (sps.zip τs).foldl (fun acc (sp, t) => acc.bind (fun (ir, bs) =>
             (subPatBinds? m t sp).map (fun (ir2, bs2) => (ir && ir2, bs ++ bs2)))) (some (true, []))
         else none                                            -- arity mismatch → decline
       else none
     | some hp, .record fields =>
       if hp == "record".toUTF8 then
         let fps := (pc.extract 1 pc.size).toList
         match fps.foldl (fun acc fp => acc.bind (fun xs => (Eval.recordField? m fp).map (fun kp => xs ++ [kp]))) (some []) with
         | none => none
         | some kps =>
           let patKeys := kps.map (·.1)
           if patKeys.eraseDups.length == patKeys.length
              && patKeys.all (fun k => fields.any (·.1 == k))
              && fields.all (fun f => patKeys.any (· == f.1)) then
             kps.foldl (fun acc (k, sp) => acc.bind (fun (ir, bs) =>
               match (fields.find? (·.1 == k)).map (·.2) with
               | some t => (subPatBinds? m t sp).map (fun (ir2, bs2) => (ir && ir2, bs ++ bs2))
               | none => none)) (some (true, []))
           else none                                          -- subset / extra key / open-row rest → decline
       else none
     | some hp, .sum variants =>
       -- T1.52 inc-5 — a NESTED VARIANT pattern `(Ctor p)` / `(Ctor)` under a match position (e.g.
       -- `#tuple(a (Some b))`): binds the payload; REFUTABLE unless the sum has a single variant. A refutable
       -- sub-pattern makes the ENCLOSING arm refutable, so the outer match needs a covering (catch-all) arm.
       (match variants.find? (·.1 == hp) with
        | some (_, some payloadTy) =>                          -- payload-bearing variant `(Ctor p)`
          if pc.size == 2 then
            (match pc[1]? with
             | some subId => (subPatBinds? m payloadTy subId).map (fun (subIr, bs) => ((variants.length == 1) && subIr, bs))
             | none => none)
          else none                                           -- `(Ctor)` on a payload variant / over-applied → decline
        | some (_, none) =>                                   -- nullary variant `(Ctor)` (no payload, no binds)
          if pc.size == 1 then some (variants.length == 1, []) else none
        | none => none)                                       -- not a variant of this sum → decline
     | _, _ => none)                                          -- a nested list pattern / literal / head-type mismatch → decline (defer)
  | none => none

/-- Dedup a bind list: `none` if a name repeats (rcdzc CDZ0201 — declined to avoid a false accept). -/
def noDupBinds? (bs : List (ByteArray × Ty)) : Option (List (ByteArray × Ty)) :=
  let ns := bs.map (·.1); if ns.eraseDups.length == ns.length then some bs else none

/-- Classify a MATCH pattern `patId` against a TUPLE scrutinee of element types `τs` (T1.52 inc-1, extended
inc-4 with NESTED sub-patterns). A `_` / bare binder binds the whole tuple; a `(tuple p1…pn)` binds each
element (each sub-pattern may itself be a nested tuple/record via `subPatBinds?`). Always IRREFUTABLE ⇒ the
match is exhaustive. `none` (→ DECLINE) on arity mismatch, a refutable/unmodeled sub-pattern, or a dup binder. -/
def tuplePatClassify? (m : Ast.Module) (τs : List Ty) (patId : Nat) : Option (Bool × List (ByteArray × Ty)) :=
  (subPatBinds? m (.tuple τs) patId).bind (fun (ir, bs) => (noDupBinds? bs).map (fun bs' => (ir, bs')))

/-- Classify a MATCH pattern `patId` against a RECORD scrutinee of fields `fields` (T1.52 inc-2). Returns
`some (irrefutable, binds)` for a MODELED pattern: a `_` / bare binder (binds the whole record —
irrefutable), or a `(record (= k1 p1)…(= kn pn))` whose field-key set EXACTLY matches the record's
(closed record, no subset/extra/rest) and whose every sub-pattern is a plain binder / `_` (each binds its
field's type — irrefutable). `none` (→ DECLINE, never a false reject) for anything else: a subset/extra
key, an open-row `(.. rest)`, a nested/literal sub-pattern, or a DUPLICATE binder. A record has a single
shape, so a modeled record pattern is always irrefutable ⇒ the match is exhaustive. -/
def recordPatClassify? (m : Ast.Module) (fields : List (ByteArray × Ty)) (patId : Nat) : Option (Bool × List (ByteArray × Ty)) :=
  (subPatBinds? m (.record fields) patId).bind (fun (ir, bs) => (noDupBinds? bs).map (fun bs' => (ir, bs')))

/-- Classify a MATCH pattern `patId` against a LIST scrutinee of element type `elem` (T1.52 inc-3). Returns
`some (coversAll, binds)`: `coversAll` = the pattern matches a list of ANY length (a `_` / bare binder, or a
whole-list rest `#list((.. rest))`), else it matches only a specific length (fixed-arity `#list(p1…pn)`) or a
minimum length (leading + rest `#list(p1…pk (.. rest))`). Each element sub-pattern binds `elem`; a trailing
grouped `(.. rest)` binds `(List elem)`. `none` (→ DECLINE, never a false reject) for a nested/literal
sub-pattern or a duplicate binder. UNLIKE tuple/record, a list is REFUTABLE (variable length), so the caller
treats a match with NO `coversAll` arm as not-provably-exhaustive and DECLINES (rather than asserting a
possibly-false CDZ0210) — the sound first cut; the accept path (some arm covers all lengths) is modeled. -/
def listPatClassify? (m : Ast.Module) (elem : Ty) (patId : Nat) : Option (Bool × List (ByteArray × Ty)) :=
  let binderName? : Nat → Option (Option ByteArray) := fun sp =>
    match m.nodes[sp]? with
    | some (.atom lid) => (match m.leaves[lid]? with
                           | some (.name b) => some (if b == "_".toUTF8 then none else some b)
                           | _ => none)
    | _ => none
  -- an element sub-pattern binds via `subPatBinds?` (T1.52 inc-4): a `_`/binder, OR a nested irrefutable
  -- tuple/record (`#list(#tuple(a b) …)`); a refutable/unmodeled element → decline.
  let addBind : List (ByteArray × Ty) → Nat → Ty → Option (List (ByteArray × Ty)) := fun bs sp τ =>
    (subPatBinds? m τ sp).map (fun (_, bs2) => bs ++ bs2)  -- list arm ignores element refutability (coversAll is length-based)
  let noDup : List (ByteArray × Ty) → Option (List (ByteArray × Ty)) := fun bs =>
    let ns := bs.map (·.1); if ns.eraseDups.length == ns.length then some bs else none
  match m.nodes[patId]? with
  | some (.atom _) =>
    (match binderName? patId with
     | some none => some (true, [])
     | some (some b) => some (true, [(b, .listTy elem)])   -- bare binder: whole list, covers all lengths
     | none => none)
  | some (.list pc) =>
    if m.headName? (.list pc) == some "list".toUTF8 then
      let sps := (pc.extract 1 pc.size).toList
      -- a trailing GROUPED `(.. rest)` (last sub-pattern headed by `..`) → leading + rest
      let restInfo : Option (List Nat × Nat) :=
        match sps.reverse with
        | last :: leadRev =>
          (match m.nodes[last]? with
           | some (.list lc) => if m.headName? (.list lc) == some "..".toUTF8 then (lc[1]?).map (fun rb => (leadRev.reverse, rb)) else none
           | _ => none)
        | [] => none
      match restInfo with
      | some (leadPats, rb) =>
        match leadPats.foldl (fun acc sp => acc.bind (fun bs => addBind bs sp elem)) (some []) with
        | none => none
        | some leadBinds =>
          (match binderName? rb with
           | some none => (noDup leadBinds).map (fun bs => (leadPats.isEmpty, bs))                       -- rest = `_`
           | some (some rname) => (noDup (leadBinds ++ [(rname, .listTy elem)])).map (fun bs => (leadPats.isEmpty, bs))
           | none => none)                                                                              -- non-binder rest → decline
      | none =>
        -- fixed-arity `#list(p1…pn)` (n may be 0) — matches ONLY length n → coversAll = false
        match sps.foldl (fun acc sp => acc.bind (fun bs => addBind bs sp elem)) (some []) with
        | none => none
        | some binds => (noDup binds).map (fun bs => (false, bs))
    else none                                            -- non-list pattern head → decline (inc-3 = list only)
  | none => none

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

/-- A HINDLEY-MILNER TYPE SCHEME: a body type universally quantified over a set of GENERAL type-var ids.
A monomorphic binding is `([], τ)` (nothing quantified — a `fn`/`match`/λ-bound name). Only GENERAL vars
(`.var`) are ever quantified — never a `.numVar` (a width-polymorphic int literal that defaults to `Int64`,
not a real type var). The environment binds names to schemes so a `let`-bound definition can be used at
different instantiations (spec type-system.md "A Let-Bound Definition Is Generalized"). -/
abbrev Scheme := List Nat × Ty

/-- The GENERAL type-var ids (`.var`) free in a type — for generalization and the not-free-in-env check.
Excludes `.numVar` (never generalized). -/
partial def freeGenVars : Ty → List Nat
  | .var i => [i]
  | .fn d c => freeGenVars d ++ freeGenVars c
  | .tuple es => es.flatMap freeGenVars
  | .listTy e => freeGenVars e
  | .setTy e => freeGenVars e
  | .mapTy k v => freeGenVars k ++ freeGenVars v
  | .record fs => fs.flatMap (fun f => freeGenVars f.2)
  | .sum vs => vs.flatMap (fun v => match v.2 with | some t => freeGenVars t | none => [])
  | _ => []

/-- Free general vars of a scheme = free vars of its body MINUS its quantified vars. -/
def schemeFreeGenVars (s : Scheme) : List Nat := (freeGenVars s.2).filter (fun v => !s.1.contains v)

/-- Resolve a scheme's body under a substitution (its quantified vars are fresh ids never bound by `s`). -/
def schemeApplySubst (subst : Subst) (s : Scheme) : Scheme := (s.1, applySubst subst s.2)

/-- The general vars free anywhere in the environment (after `subst`) — the vars a `let` binding may NOT
generalize, since they are still constrained by an enclosing binding (spec type-system.md §"constrained"). -/
def envFreeGenVars (subst : Subst) (env : List (ByteArray × Scheme)) : List Nat :=
  env.flatMap (fun e => schemeFreeGenVars (schemeApplySubst subst e.2))

/-- INSTANTIATE a scheme at a use site: allocate a fresh general var per quantified id and substitute,
yielding a fresh monomorphic copy (each use gets its own vars — this is what makes the binding polymorphic). -/
def instantiateScheme (s : Scheme) (st : InferState) : Ty × InferState :=
  let (sub, st') := s.1.foldl (fun (acc : Subst × InferState) qv =>
      ((qv, Ty.var acc.2.next) :: acc.1, { acc.2 with next := acc.2.next + 1 })) (([] : Subst), st)
  (applySubst sub s.2, st')

/-- GENERALIZE a type at a `let` / `do`-def binding (spec type-system.md "A Let-Bound Definition Is
Generalized"): apply the current subst, then quantify the general vars free in the type but NOT free in the
environment (a var still constrained by an enclosing binding MUST NOT be generalized, else generalization
escapes the scope in which it is still being solved). -/
def generalizeScheme (env : List (ByteArray × Scheme)) (subst : Subst) (τ : Ty) : Scheme :=
  let τr := applySubst subst τ
  let envFree := envFreeGenVars subst env
  ((freeGenVars τr).eraseDups.filter (fun v => !envFree.contains v), τr)

/-- Wrap a monomorphic type as a scheme with nothing quantified (a `fn`/`match`/λ-bound name). -/
def monoScheme (τ : Ty) : Scheme := ([], τ)

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
  element type of `base`'s tuple; out-of-arity or a non-tuple base is `IllTyped CDZ0203`.
* T1.13 — **closed record** construction (`ts:70-74`): `(record (= k v)…)` → `.record [(k,τ)…]` sorted by
  key; a duplicate field is `IllTyped CDZ0211`.
* T1.14 — **record field access** (`. r f`, `ts:104-108`): `f`'s type from `r`'s record; an absent field
  is `CDZ0212`, a field access on a non-record `CDZ0203`.
* T1.15 — **built-in sum construction** (`ts:192-204`): `(Some x)`→`Option τx`, `(None)`→`Option α`,
  `(Ok x)`/`(Err x)`→`Result`, `(Less)/(Equal)/(Greater)`→`Ordering`. Constructors are uniformly
  single-arity (spec core-semantics.md): a "nullary" variant (`None`/`Less`/…) is a Unit-payload
  constructor, so `(None unit)` (canonical) and bare `(None)` (sugar) are both fine — one optional
  payload arg. Over-applying — a payload-bearing ctor with a surplus arg (`(Some x y)`, size > 2), or a
  Unit-payload ctor with two (`(None u v)`, size > 2) — is `IllTyped CDZ0203`: the surplus arg applies a
  complete (non-function) Sum value, never a silent drop. An UNDETERMINED sum at the escape (`ts:34`,
  e.g. bare `None` as the sole result) is declined by `inferBody`'s `hasUndeterminedSum` guard (Unsupported).
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
* T1.12/T1.18 — **App** (`ts:36`): `(f a…)` with `f` a name bound to a scheme — INSTANTIATE it (fresh
  vars per use), then unify the arrow against each arg to a fresh result var → the codomain; a domain
  clash / non-fn head is `CDZ0203`. An unbound head (a prelude/builtin) → `Unsupported`.
* T1.18 — **let-generalization**: a `let`/`do`-bound definition is generalized over its free general vars
  not free in the env (spec "A Let-Bound Definition Is Generalized"), so it may be used at several
  instantiations; `fn`/`match`/λ-bound names stay monomorphic.
* T1.16 — **Mat** (`match`): `(match scrut (pat body)…)` over a built-in sum — infer the scrutinee to a
  concrete sum, classify each arm's pattern (variant + payload binder, or catch-all) via
  `matchPatClassify?`, bind payloads, and unify all arm bodies to one result type. Non-sum scrutinee /
  unmodeled pattern → `Unsupported`; arm-body type clash → `CDZ0203`; a modeled match omitting a variant
  with no catch-all → `CDZ0210` NonExhaustive (T1.17).
Any other construct → `Unsupported` until its rule lands. -/
partial def inferE (m : Ast.Module) (env : List (ByteArray × Scheme)) (st : InferState) (nodeId : Nat) :
    Except InferFail (Ty × InferState) :=
  match scalarLitTy? m nodeId with
  | some (.int _ _) =>                                    -- OQ-G: an int LITERAL occurrence is width-polymorphic
    .ok (.numVar st.next, { st with next := st.next + 1 })  -- → a fresh numeric var, resolved by use/ascription
  | some (.float _) =>                                   -- a FLOAT literal is width-polymorphic too (T1.40)
    .ok (.floatVar st.next, { st with next := st.next + 1 }) -- → a fresh float var, resolved by use/ascription (default Float64)
  | some τ => .ok (τ, st)
  | none =>
    match Eval.nameOf? m nodeId with
    | some nm =>
      match env.find? (fun e => e.1 == nm) with
      | some (_, sch) => let (τ, st') := instantiateScheme sch st; .ok (τ, st')   -- V rule: instantiate the scheme (fresh vars per use)
      | none =>
        -- a bare NULLARY built-in constructor as an atom: `Less`/`Equal`/`Greater` → Ordering; `None` →
        -- `Option α` (fresh payload var). Only on an env-miss (a local binding shadows).
        match orderingCtorTy? nm with
        | some ot => .ok (ot, st)
        | none =>
          if nm == "None".toUTF8 then .ok (optionTy (.var st.next), { st with next := st.next + 1 })
          else
            -- T1.26 — a bare NULLARY user variant `(type C R G B)` used in value position (`R`) constructs
            -- its type's sum. Only NULLARY variants (payload `none`) — a bare payload-bearing variant name
            -- is a partial constructor (a function), declined. Env-miss only ⇒ scope-first respected
            -- (a def/param/let binding of the same name resolves via `env.find?` above).
            (match (userSumMap m).bind (fun mp => mp.find? (fun e => e.1 == nm)) with
             | some (_, (_, sumTy, none)) => .ok (sumTy, st)
             | _ => .error (.unsupported
                 "type oracle: unresolved name (may be a prelude/builtin or local binder — CDZ0101 unbound needs the prelude scope model)"))
    | none =>
      match m.nodes[nodeId]? with
      | some (.list children) =>
        match m.headName? (.list children) with
        | some h =>
          if h == "if".toUTF8 && children.size == 4 then      -- exactly (if c t e); wrong arity → App→Unsupported
            match children[1]?, children[2]?, children[3]? with
            | some cId, some tId, some eId => do
                let (τc, st) ← inferE m env st cId
                let st ← unifyInfer τc .bool st          -- condition must be Bool (ts:76)
                let (τt, st) ← inferE m env st tId
                let (τe, st) ← inferE m env st eId
                let st ← unifyInfer τt τe st              -- both branches unify; never absorbs (ts:82-84)
                .ok (applySubst st.subst τt, st)
            | _, _, _ => .error (.unsupported "type oracle: malformed if")
          else if children.size == 3 && (String.fromUTF8? h).elim false (fun s => Eval.cmpOps.contains s || s == "=") then
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
                let τ := applySubst st.subst τa
                -- a FUNCTION is not comparable/equatable (ts) — `(= f g)` / `(< f g)` is a type error.
                match τ with
                | .fn _ _ => .error (.illTyped "CDZ0203")
                | _ =>
                  -- EQUALITY (`=`) works on ANY non-fn value (canonical byte-form equality — floats/sets/maps
                  -- and compounds containing them are all equatable). ORDERING (`< > <= >=`) needs a TOTAL
                  -- order: a BARE scalar float is fine (IEEE partial order → Bool), but a SET/MAP, or any
                  -- COMPOUND containing a float/set/map, has NO total order → CDZ0203 (03-equality:1017 —
                  -- "a compound is ordered only when EVERY component is"; the float/set/map no-total-order family).
                  if h == "=".toUTF8 then .ok (.bool, st)
                  else match τ with
                       | .float _ | .floatVar _ => .ok (.bool, st)
                       | _ => if containsSetOrMap τ then .error (.illTyped "CDZ0203") else .ok (.bool, st)
            | _, _ => .error (.unsupported "type oracle: malformed comparison")
          else if children.size == 3 && (String.fromUTF8? h).elim false (fun s => Eval.arithOps.contains s) then
            -- T1.4 — ARITHMETIC (`+ - * / %`): `(OP a b)` unifies the two operands, then requires the result
            -- to be NUMERIC — `Int` → result that int type; a same-typed NON-numeric operand (`Bool`/`String`/
            -- `Char`/`Unit`) is `IllTyped CDZ0301` NumericMismatch (§4). A mixed operand clash was already
            -- caught by the unify (`CDZ0203`). `never` absorbs (`(+ (trap) x)`). SOUND on float: a float
            -- operand (`.float`/`.floatVar`, now modeled — T1.40) falls to the `_` catch-all → `Unsupported`
            -- (a SKIP), never a false `Int`-reject — float ARITHMETIC is a deliberate later increment (needs
            -- per-op validity, e.g. `%`-on-float). A still-unresolved operand type → `Unsupported` too.
            match children[1]?, children[2]? with
            | some aId, some bId => do
                let (τa, st) ← inferE m env st aId
                let (τb, st) ← inferE m env st bId
                -- T1.46 — BigInt does NOT silently promote a bare literal in ARITHMETIC (06-numeric:0394): a
                -- (bigint, numVar-literal) mix is CDZ0301 — EVEN THOUGH comparison/ascription/`of` DO ground
                -- a literal to bigint (via the numVar↔bigint unify arm). Guard it BEFORE unify grounds it.
                match applySubst st.subst τa, applySubst st.subst τb with
                | .bigint, .numVar _ | .numVar _, .bigint => .error (.illTyped "CDZ0301")
                | _, _ => do
                  let st ← unifyInfer τa τb st
                  match applySubst st.subst τa with
                  | .int w sg => .ok (.int w sg, st)
                  | .numVar _ => .ok (.int 64 true, st)     -- unconstrained int literal(s) → numeric, default Int
                  -- T1.41 — FLOAT arithmetic: `+ - * /` on floats → the float type (kept width-POLY: return the
                  -- resolved operand, so `(: (+ 1.0 2.0) Float32)` still resolves; a lingering floatVar defaults
                  -- to Float64 at escape). `%` (remainder) on a float is CDZ0301 — floats have NO remainder
                  -- (18-units-of-measure:2456 "no float remainder"; exact/float division is total). Matches rcdzc.
                  | .float w => if h == "%".toUTF8 then .error (.illTyped "CDZ0301") else .ok (.float w, st)
                  | .floatVar i => if h == "%".toUTF8 then .error (.illTyped "CDZ0301") else .ok (.floatVar i, st)
                  -- T1.44 — RATIONAL arithmetic: `+ - * /` → Rational (exact). `%` → CDZ0301 (no remainder on
                  -- exact/floating arithmetic — 18-units-of-measure:2429, same carve-out as float).
                  | .rational => if h == "%".toUTF8 then .error (.illTyped "CDZ0301") else .ok (.rational, st)
                  -- T1.46 — BIGINT arithmetic: `+ - * / %` ALL yield BigInt (it is an INTEGER — remainder IS
                  -- defined, unlike float/rational — and arbitrary precision never traps/overflows).
                  | .bigint => .ok (.bigint, st)
                  | .never => .ok (.never, st)
                  | .bool | .string | .char | .unit => .error (.illTyped "CDZ0301")
                  | _ => .error (.unsupported "type oracle: arithmetic on an unresolved/unmodeled operand type")
            | _, _ => .error (.unsupported "type oracle: malformed arithmetic (unary or partial)")
          else if (h == "not".toUTF8 && children.size == 2)
               || ((h == "and".toUTF8 || h == "or".toUTF8) && children.size == 3) then
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
          else if h == "list".toUTF8 then
            -- T1.29 — LIST construction `(list e…)`: infer each element and UNIFY them to one element type
            -- (a `List` is homogeneous) → `.listTy τ`. A non-homogeneous element clash is `IllTyped CDZ0203`.
            -- An EMPTY `(list)` has an unconstrained element type (needs context we don't model) → declined.
            let elemIds := children.extract 1 children.size
            if elemIds.isEmpty then .error (.unsupported "type oracle: empty (list) — element type unconstrained, declined")
            else (match elemIds.foldlM (m := Except InferFail)
                (fun (acc : Ty × InferState) eid =>
                  match inferE m env acc.2 eid with
                  | .ok (τ, st') => (match unifyInfer acc.1 τ st' with
                                     | .ok st'' => .ok (acc.1, st'')
                                     | .error e => .error e)
                  | .error e => .error e)
                (.var st.next, { st with next := st.next + 1 }) with
             | .ok (τelem, st') => .ok (.listTy (applySubst st'.subst τelem), st')
             | .error e => .error e)
          else if h == "set".toUTF8 then
            -- T1.32 — SET construction `(set e…)`: like `(list e…)` — infer each element and UNIFY them to
            -- one element type (a `Set` is homogeneous) → `.setTy τ`. A clash is `IllTyped CDZ0203`; an
            -- EMPTY `(set)` has an unconstrained element type → declined (sound, like empty list).
            let elemIds := children.extract 1 children.size
            if elemIds.isEmpty then .error (.unsupported "type oracle: empty (set) — element type unconstrained, declined")
            else (match elemIds.foldlM (m := Except InferFail)
                (fun (acc : Ty × InferState) eid =>
                  match inferE m env acc.2 eid with
                  | .ok (τ, st') => (match unifyInfer acc.1 τ st' with
                                     | .ok st'' => .ok (acc.1, st'')
                                     | .error e => .error e)
                  | .error e => .error e)
                (.var st.next, { st with next := st.next + 1 }) with
             | .ok (τelem, st') => .ok (.setTy (applySubst st'.subst τelem), st')
             | .error e => .error e)
          else if h == "map".toUTF8 then
            -- T1.33 — MAP construction `(map (= k v)…)`: each entry is a field-pair `(= keyExpr valExpr)`
            -- (the SAME field-pair form records use, but the KEY is an expression, not a name). Infer every
            -- key and unify them to one key type K, every value to one value type V → `.mapTy K V`. A key or
            -- value clash is `IllTyped CDZ0203`; a non-`(= _ _)` entry declines; an EMPTY `(map)` has
            -- unconstrained K/V → declined (sound, like empty list/set).
            -- PURE pass first: flatten each `(= keyExpr valExpr)` entry to two `(isKey, nodeId)` slots. A
            -- malformed / non-field-pair entry → `none` (declines). This keeps the inference fold's closure
            -- calling `inferE` exactly ONCE per element (a closure that calls `inferE` TWICE defeats the
            -- compiler's `Array.foldlMUnsafe` specializer → a `uses sorry` codegen failure).
            let entryIds := children.extract 1 children.size
            if entryIds.isEmpty then .error (.unsupported "type oracle: empty (map) — key/value types unconstrained, declined")
            else (match entryIds.foldl (init := (some #[] : Option (Array (Bool × Nat))))
                (fun acc eid => acc.bind (fun arr =>
                  match m.nodes[eid]? with
                  | some (.list fc) =>
                    if m.headName? (.list fc) == some "=".toUTF8 && fc.size == 3 then
                      some ((arr.push (true, fc[1]!)).push (false, fc[2]!))
                    else none
                  | _ => none)) with
             | none => .error (.unsupported "type oracle: map entry not a (= key value) field-pair")
             | some flat =>
               let K : Ty := .var st.next
               let V : Ty := .var (st.next + 1)
               let st0 := { st with next := st.next + 2 }
               (match flat.foldlM (m := Except InferFail)
                   (fun (acc : InferState) (p : Bool × Nat) =>
                     match inferE m env acc p.2 with
                     | .ok (τ, st1) => unifyInfer τ (if p.1 then K else V) st1
                     | .error e => .error e) st0 with
                | .ok st' => .ok (.mapTy (applySubst st'.subst K) (applySubst st'.subst V), st')
                | .error e => .error e))
          else if h == "record".toUTF8 then
            -- T1.13 — CLOSED RECORD construction (`ts:70-74`): `(record (= k v)…)` infers each field value,
            -- sorts fields by key (canonical form, so `unify` compares field SETs), and yields `.record`.
            -- A DUPLICATE field name is `IllTyped CDZ0211` PresentField. An ill-typed/unsupported field
            -- value propagates.
            (match (children.extract 1 children.size).foldlM (m := Except InferFail)
                (fun (acc : List (ByteArray × Ty) × InferState) fid =>
                  match Eval.recordField? m fid with
                  | some (k, vId) =>
                    -- a record SPREAD `(.. r)` mis-parses as a field literally named ".." (no valid field is
                    -- named "..") — the oracle doesn't model spread (merge r's fields + overrides), so
                    -- DECLINE the whole record → Unsupported (sound: never a false absent-field on a spread).
                    if k == "..".toUTF8 then
                      .error (.unsupported "type oracle: record spread (.. r) not modeled — declined")
                    else (match inferE m env acc.2 vId with
                          | .ok (τ, st') => .ok (acc.1 ++ [(k, τ)], st')
                          | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed record field")) ([], st) with
             | .ok (fields, st') =>
               let sorted := (fields.toArray.qsort (fun a b => Eval.cmpBytes a.1 b.1 == .lt)).toList
               if (sorted.zip (sorted.drop 1)).any (fun (p : (ByteArray × Ty) × (ByteArray × Ty)) => p.1.1 == p.2.1) then
                 .error (.illTyped "CDZ0211")           -- duplicate field name
               else .ok (.record sorted, st')
             | .error e => .error e)
          else if h == ".".toUTF8 && children.size == 3 then
            -- T1.7 — positional TUPLE PROJECTION `(. base i)` where `i` is an INT literal (`ts:146`): infer
            -- `base` as a tuple, then the `i`-th element type if `i < arity`, else out-of-arity is
            -- `IllTyped CDZ0203`; a positional projection of a NON-tuple is also a type error (`CDZ0203`).
            -- A NAME index (record-field / module-member access) is `Unsupported` until records/members are
            -- modeled — a name index never decodes to an int (`Value.ofLeaf`), so it falls through cleanly.
            match children[1]?, children[2]? with
            | some baseId, some idxId =>
              match (m.nodes[idxId]?).bind (fun n => match n with | .atom lid => m.leaves[lid]? | _ => none) with
              | some (Ast.Leaf.name fld) =>
                -- T1.35 — `Map.empty` (`(. Map empty)`): the polymorphic empty map → `.mapTy (fresh)(fresh)`.
                -- Used in context (`(Map.insert Map.empty k v)`) its k/v unify to the surrounding types; a
                -- BARE escaping empty map stays undetermined and is declined by inferBody's undetermined-
                -- collection escape guard (sound — rcdzc cannot determine a bare empty map's type either).
                if (Eval.nameOf? m baseId == some "Map".toUTF8) && fld == "empty".toUTF8 then
                  .ok (.mapTy (.var st.next) (.var (st.next + 1)), { st with next := st.next + 2 })
                -- T1.42 — the float CONSTANT `nan` of a width: `(. Float64 nan)` → Float64, `(. Float32 nan)` →
                -- Float32 (annotated to THIS width — a cross-width `(= Float32.nan Float64.nan)` still clashes).
                else if fld == "nan".toUTF8 && Eval.nameOf? m baseId == some "Float64".toUTF8 then .ok (.float 64, st)
                else if fld == "nan".toUTF8 && Eval.nameOf? m baseId == some "Float32".toUTF8 then .ok (.float 32, st)
                -- T1.43 — int-module bound CONSTANTS `(. Int64 max)` / `(. UInt8 min)` → that width's `.int w s`.
                else if (fld == "max".toUTF8 || fld == "min".toUTF8) &&
                        ((Eval.nameOf? m baseId).bind intWidthName?).isSome then
                  (match (Eval.nameOf? m baseId).bind intWidthName? with
                   | some (w, s) => .ok (.int w s, st)
                   | none => .error (.unsupported "type oracle: int bound"))  -- unreachable (guarded above)
                else
                -- T1.27 — QUALIFIED nullary variant `(. Q M)` = `Q.M` (a `(type Q … M …)` variant used
                -- unapplied): if `fld` is a NULLARY variant AND its declaring type name equals the base name
                -- `Q`, construct that type's sum. (A payload-bearing `Q.M` unapplied is a partial ctor →
                -- falls through to the field-access path → declines.) Checked BEFORE record field access.
                match (Eval.nameOf? m baseId).bind (fun q =>
                        (userSumMap m).bind (fun mp =>
                          (mp.find? (fun e => e.1 == fld)).bind (fun ent =>
                            if ent.2.1 == q then some ent else none))) with
                | some (_, (_, sumTy, none)) => .ok (sumTy, st)
                | _ =>
                -- T1.14 — record FIELD ACCESS `(. r f)` (`ts:104-108`): infer `r` as a record, then `f`'s
                -- type; an absent field is `IllTyped CDZ0212`, a field access on a non-record is `CDZ0203`.
                -- A base that isn't a modeled record (e.g. a module member like `(. Float64 nan)` whose base
                -- name doesn't resolve) infers `Unsupported` and propagates — never a false field access.
                -- T1.37 — NEWTYPE ERASURE: `eraseNewtypes` sees through a single-variant newtype tag (e.g.
                -- `(UserId.Mk record).x` reads the payload record's field — 05-compound-types:8519) to the
                -- payload record; a bare record is unchanged (erasure is a no-op on non-newtypes).
                (match inferE m env st baseId with
                 | .ok (τb, st') =>
                   (match eraseNewtypes (applySubst st'.subst τb) with
                    | .record fields => (match fields.find? (fun f => f.1 == fld) with
                                         | some (_, τ) => .ok (τ, st')
                                         | none => .error (.illTyped "CDZ0212"))
                    | .never => .ok (.never, st')
                    | _ => .error (.illTyped "CDZ0203"))
                 | .error e => .error e)
              | some l =>
                (match Value.ofLeaf l with
                 | some (.int n) =>
                   if n < 0 then .error (.illTyped "CDZ0203")
                   else do
                     let (τb, st) ← inferE m env st baseId
                     -- T1.37 newtype erasure applies to POSITIONAL projection too (a newtype over a tuple).
                     match eraseNewtypes (applySubst st.subst τb) with
                     | .tuple τs => (match τs[n.toNat]? with
                                     | some τ => .ok (τ, st)
                                     | none => .error (.illTyped "CDZ0203"))
                     | .never => .ok (.never, st)
                     | _ => .error (.illTyped "CDZ0203")
                 | _ => .error (.unsupported "type oracle: unmodeled projection index"))
              | none => .error (.unsupported "type oracle: malformed projection")
            | _, _ => .error (.unsupported "type oracle: malformed projection")
          else if h == "let".toUTF8 then
            -- T1.8/T1.18 — LET (`ts:40-44`), now with HM GENERALIZATION: `(let ((x e)…) body)` infers each
            -- binding value and extends the env with `x : GENERALIZE(τ)` SEQUENTIALLY (a later binding sees
            -- the earlier), then infers the body under the extended env. Each binding's type is generalized
            -- over its free general vars NOT free in the env (spec type-system.md "A Let-Bound Definition Is
            -- Generalized"), so a polymorphic `x` (e.g. `(fn (a) a) : ∀α.α→α`) may be used at several
            -- instantiations in the body (the V rule instantiates fresh vars per use). An `IllTyped`/
            -- `Unsupported` binding value propagates.
            match children[1]?, children[2]? with
            | some bindingsId, some bodyId =>
              (match m.nodes[bindingsId]? with
               | some (Ast.Node.list pairs) =>
                 (match pairs.foldlM (m := Except InferFail) (fun (acc : List (ByteArray × Scheme) × InferState) pid =>
                     match m.nodes[pid]? with
                     | some (Ast.Node.list pc) =>
                       (match pc[0]?, pc[1]? with
                        | some nId, some vId =>
                          (match Eval.nameOf? m nId with
                           | some nm =>
                             (match inferE m acc.1 acc.2 vId with
                              | .ok (τ, st') => .ok (((nm, generalizeScheme acc.1 st'.subst τ) :: acc.1), st')
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
              (match stmts.foldlM (m := Except InferFail) (fun (acc : List (ByteArray × Scheme) × InferState) sid =>
                  match Eval.asDef? m sid with
                  | some dc =>
                    (match dc[1]?, dc[dc.size - 1]? with
                     | some targetId, some valId =>
                       (match Eval.nameOf? m targetId with
                        | some nm =>                       -- value def → bind x : GENERALIZE(τ) (like let)
                          (match inferE m acc.1 acc.2 valId with
                           | .ok (τ, st') => .ok (((nm, generalizeScheme acc.1 st'.subst τ) :: acc.1), st')
                           | .error e => .error e)
                        | none =>
                          -- T1.21 — LOCAL FN-DEF `(def (f p…) body)`: target is a list `(f p…)`. Type like the
                          -- Fn rule (each param → fresh var / annotated type, infer body, curried arrow), then
                          -- GENERALIZE + bind `f` (like `let`). 🪤 RECURSION: `f` is bound only AFTER the body,
                          -- so a body referencing `f` → unbound → `Unsupported` (declines; sound skip — recursive
                          -- local fns are a later increment). An unmodeled param annotation → Unsupported.
                          (match m.nodes[targetId]? with
                           | some (Ast.Node.list tc) =>
                             (match (tc[0]?).bind (Eval.nameOf? m) with
                              | some fnm =>
                                (match (tc.extract 1 tc.size).foldlM (m := Except InferFail)
                                    (fun (pacc : List (ByteArray × Scheme) × List Ty × InferState) pid =>
                                      match m.nodes[pid]? with
                                      | some (Ast.Node.atom plid) =>
                                        (match m.leaves[plid]? with
                                         | some (.name pnm) =>
                                           let α : Ty := .var pacc.2.2.next
                                           .ok ((pnm, ([], α)) :: pacc.1, α :: pacc.2.1, { pacc.2.2 with next := pacc.2.2.next + 1 })
                                         | _ => .error (.unsupported "type oracle: malformed local-fn param"))
                                      | some (Ast.Node.list ppc) =>            -- (: name T)
                                        (match ppc[1]?, ppc[2]? with
                                         | some pnId, some ptId =>
                                           (match Eval.nameOf? m pnId, parseTy? m ptId with
                                            | some pnm, some pτ => .ok ((pnm, ([], pτ)) :: pacc.1, pτ :: pacc.2.1, pacc.2.2)
                                            | some _, none => .error (.unsupported "type oracle: local-fn param unmodeled annotation")
                                            | none, _ => .error (.unsupported "type oracle: local-fn param missing name"))
                                         | _, _ => .error (.unsupported "type oracle: malformed local-fn param spec"))
                                      | none => .error (.unsupported "type oracle: malformed local-fn param"))
                                    (acc.1, [], acc.2) with
                                 | .ok (bodyEnv, ptysRev, stP) =>
                                   -- T1.22 — MONOMORPHIC RECURSION: pre-bind `fnm : (params → fresh ρ)` MONO
                                   -- so the body may call itself, infer the body, then unify its type with ρ.
                                   -- Generalize the resolved arrow (let-gen at the def). Non-recursive defs are
                                   -- unaffected (fnm unused → ρ := bodyτ → same arrow as before).
                                   -- T1.24 SOUNDNESS (as topLevelEnv): duplicate param → decline (rcdzc
                                   -- CDZ0102); body ill-typed in isolation → decline (monomorphization-
                                   -- dependent), never a positive reject.
                                   let pnames := (bodyEnv.take ptysRev.length).map (·.1)
                                   if pnames.eraseDups.length != pnames.length then
                                     .error (.unsupported "type oracle: duplicate local-fn param name — declined (rcdzc CDZ0102)")
                                   else if ptysRev.any (fun τ => match τ with | .fn _ _ => true | _ => false) then
                                     .error (.unsupported "type oracle: higher-order local fn-def (function-typed param) — context/monomorphization-dependent, declined")
                                   else
                                     let ρ : Ty := .var stP.next
                                     let recArrow := ptysRev.foldl (fun a pτ => Ty.fn pτ a) ρ
                                     let stP1 := { stP with next := stP.next + 1 }
                                     (match inferE m ((fnm, ([], recArrow)) :: bodyEnv) stP1 valId with
                                      | .ok (bodyτ, stB) =>
                                        (match unifyInfer bodyτ ρ stB with
                                         | .ok stB2 =>
                                           let arrow := ptysRev.foldl (fun a pτ => Ty.fn pτ a) ρ
                                           .ok (((fnm, generalizeScheme acc.1 stB2.subst arrow) :: acc.1), stB2)
                                         | .error _ => .error (.unsupported "type oracle: local fn-def body ill-typed in isolation — monomorphization-dependent, declined"))
                                      | .error (.illTyped _) => .error (.unsupported "type oracle: local fn-def body ill-typed in isolation — monomorphization-dependent, declined")
                                      | .error (.unsupported r) => .error (.unsupported r))
                                 | .error e => .error e)
                              | none => .error (.unsupported "type oracle: local fn-def head not a name"))
                           | _ => .error (.unsupported "type oracle: malformed local fn-def target")))
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
                   -- T1.44 — a numeric LITERAL explicitly annotated `Rational` GROUNDS to Rational (an int
                   -- literal or a decimal/scientific literal — 06-numeric-model:0136-0141). This grounding is
                   -- ONLY at the explicit ascription: arithmetic/app do NOT silently promote (unify has no
                   -- numVar/floatVar↔rational arm, so `(+ (Rational.of 1 2) 1)` still clashes CDZ0301).
                   | .numVar _, .rational => .ok (.rational, st)
                   | .floatVar _, .rational => .ok (.rational, st)
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
                     (fun (acc : List (ByteArray × Scheme) × List Ty × InferState) pid =>
                       match m.nodes[pid]? with
                       | some (Ast.Node.atom lid) =>            -- bare param → fresh var, bound MONOMORPHIC (λ-bound)
                         (match m.leaves[lid]? with
                          | some (.name nm) =>
                            let α : Ty := .var acc.2.2.next
                            .ok ((nm, ([], α)) :: acc.1, α :: acc.2.1, { acc.2.2 with next := acc.2.2.next + 1 })
                          | _ => .error (.unsupported "type oracle: malformed fn param"))
                       | some (Ast.Node.list pc) =>            -- (: name T)
                         (match pc[1]?, pc[2]? with
                          | some nId, some tId =>
                            (match Eval.nameOf? m nId, parseTy? m tId with
                             | some nm, some τ => .ok ((nm, ([], τ)) :: acc.1, τ :: acc.2.1, acc.2.2)
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
          else if h == "Some".toUTF8 then
            -- T1.15 — built-in sum CONSTRUCTION (`ts:192-204`). `(Some x)` → `Option τx`.
            -- 🪤 ARITY: `Some` is SINGLE-arity. `(Some x y…)` desugars to `((Some x) y)` — applying the
            -- complete Option value `(Some x)` to a surplus arg is applying a non-function → `CDZ0203`
            -- (corpus 09-functions/0208-0209; a naive `children[1]?` read would silently DROP the surplus
            -- and false-accept). children = [head, arg…]; exactly one arg ⇒ size 2.
            if children.size > 2 then .error (.illTyped "CDZ0203")
            else (match children[1]? with
             | some xId => (match inferE m env st xId with
                            | .ok (τ, st') => .ok (optionTy τ, st')
                            | .error e => .error e)
             | none => .error (.unsupported "type oracle: malformed Some"))
          else if h == "None".toUTF8 then
            -- `None` is a single-arity constructor whose payload type is `Unit` (spec core-semantics.md:
            -- "uniform single-arity constructors"). Canonical construction is `(None unit)`; bare `(None)`
            -- is surface sugar for the SAME value. So one optional payload arg is fine (children ≤ 2);
            -- only a genuine over-application `(None u v)` (size > 2) is `CDZ0203`. The `Unit` payload
            -- carries no element type, so the Option element is a fresh var either way. (The arg is not
            -- type-checked against `Unit` here — declining that refinement is sound; a wrong reject is
            -- worse. Corpus 05-compound-types/0746-0747 `(= (None unit) (Some 1))` pins this.)
            if children.size > 2 then .error (.illTyped "CDZ0203")
            else .ok (optionTy (.var st.next), { st with next := st.next + 1 })   -- `Option α`, fresh payload var
          else if h == "Ok".toUTF8 then
            if children.size > 2 then .error (.illTyped "CDZ0203")               -- single-arity; over-apply → CDZ0203
            else (match children[1]? with
             | some xId => (match inferE m env st xId with
                            | .ok (τ, st') => .ok (resultTy τ (.var st'.next), { st' with next := st'.next + 1 })
                            | .error e => .error e)
             | none => .error (.unsupported "type oracle: malformed Ok"))
          else if h == "Err".toUTF8 then
            if children.size > 2 then .error (.illTyped "CDZ0203")               -- single-arity; over-apply → CDZ0203
            else (match children[1]? with
             | some xId => (match inferE m env st xId with
                            | .ok (τ, st') => .ok (resultTy (.var st'.next) τ, { st' with next := st'.next + 1 })
                            | .error e => .error e)
             | none => .error (.unsupported "type oracle: malformed Err"))
          else if (orderingCtorTy? h).isSome then
            -- `Less`/`Equal`/`Greater` are single-arity Unit-payload constructors (as None above):
            -- `(Less)` sugar for `(Less unit)`, one optional payload arg OK (size ≤ 2); over-application
            -- `(Less u v)` (size > 2) → CDZ0203.
            if children.size > 2 then .error (.illTyped "CDZ0203")
            else .ok (orderingTy, st)                        -- `(Less)`/`(Less unit)`/… → Ordering
          else if h == "match".toUTF8 then
            -- T1.16 — Mat: `(match scrut (pat body)…)` over a built-in sum. Infer the scrutinee to a
            -- CONCRETE sum type; each arm's pattern selects a variant (binding its payload via
            -- `matchPatClassify?`) or is a catch-all; all arm bodies unify to one result type. NARROW +
            -- SOUND: a non-sum scrutinee, any unmodeled pattern, or a non-exhaustive arm set → `Unsupported`
            -- (declined), never a false reject. Asserted rejects: a propagated scrutinee fault, an arm-body
            -- TYPE CLASH (`CDZ0203` via `unifyInfer`), and — T1.17 — a MODELED match that omits a variant
            -- with no catch-all → `CDZ0210` NonExhaustive (rcdzc rejects a non-exhaustive sum match, spec
            -- "#A Match Is Exhaustive Against The Sum Type's Variant Set"; a genuinely non-exhaustive sum
            -- match is never accepted, so this can only agree/holds — never a false accept).
            match children[1]? with
            | none => .error (.unsupported "type oracle: malformed match (no scrutinee)")
            | some scrutId =>
              (match inferE m env st scrutId with
               | .error e => .error e
               | .ok (τs0, st0) =>
                 (match applySubst st0.subst τs0 with
                  | .sum vs =>
                    let arms := children.extract 2 children.size
                    if arms.size == 0 then .error (.unsupported "type oracle: match with no arms")
                    else (match arms.foldlM (m := Except InferFail)
                        (fun (acc : List ByteArray × Bool × Option Ty × InferState) armId =>
                          match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                          | none => .error (.unsupported "type oracle: malformed match arm")
                          | some ac =>
                            (match ac[0]?, ac[1]? with
                             | some patId, some bodyId =>
                               (match matchPatClassify? m vs (.sum vs) patId with
                                | none => .error (.unsupported "type oracle: unmodeled match pattern — declined")
                                | some (cov, catchAll, binds) =>
                                  (match inferE m (binds.map (fun b => (b.1, ([], b.2))) ++ env) acc.2.2.2 bodyId with  -- pattern binders are monomorphic
                                   | .error e => .error e
                                   | .ok (τb, st') =>
                                     (match acc.2.2.1 with
                                      | none => .ok (acc.1 ++ cov, acc.2.1 || catchAll, some τb, st')
                                      | some τr =>
                                        (match unifyInfer τb τr st' with
                                         | .error e => .error e
                                         | .ok st'' => .ok (acc.1 ++ cov, acc.2.1 || catchAll, some τr, st'')))))
                             | _, _ => .error (.unsupported "type oracle: malformed match arm")))
                        (([], false, none, st0) : List ByteArray × Bool × Option Ty × InferState) with
                     | .error e => .error e
                     | .ok (covered, catchAll, resTy, stF) =>
                       (match resTy with
                        | none => .error (.unsupported "type oracle: match produced no result type")
                        | some τr =>
                          let exhaustive := catchAll || (vs.map (·.1)).all (fun vn => covered.any (· == vn))
                          if exhaustive then .ok (τr, stF)
                          else .error (.illTyped "CDZ0210")))    -- T1.17: a modeled match missing a variant (no catch-all) is NonExhaustive
                  | .tuple τs =>
                    -- T1.52 inc-1 — MATCH on a TUPLE scrutinee: each arm is a fixed-arity tuple pattern
                    -- (binding each element type) or a catch-all binder, classified via `tuplePatClassify?`.
                    -- A tuple has a single shape, so a modeled tuple pattern is IRREFUTABLE ⇒ exhaustive; an
                    -- unmodeled sub-pattern declines (Unsupported), never a false reject. Arm bodies unify.
                    let arms := children.extract 2 children.size
                    if arms.size == 0 then .error (.unsupported "type oracle: match with no arms")
                    else (match arms.foldlM (m := Except InferFail)
                        (fun (acc : Bool × Option Ty × InferState) armId =>
                          match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                          | none => .error (.unsupported "type oracle: malformed match arm")
                          | some ac =>
                            (match ac[0]?, ac[1]? with
                             | some patId, some bodyId =>
                               (match tuplePatClassify? m τs patId with
                                | none => .error (.unsupported "type oracle: unmodeled tuple match pattern — declined")
                                | some (irref, binds) =>
                                  (match inferE m (binds.map (fun b => (b.1, ([], b.2))) ++ env) acc.2.2 bodyId with
                                   | .error e => .error e
                                   | .ok (τb, st') =>
                                     (match acc.2.1 with
                                      | none => .ok (acc.1 || irref, some τb, st')
                                      | some τr =>
                                        (match unifyInfer τb τr st' with
                                         | .error e => .error e
                                         | .ok st'' => .ok (acc.1 || irref, some τr, st'')))))
                             | _, _ => .error (.unsupported "type oracle: malformed match arm")))
                        ((false, none, st0) : Bool × Option Ty × InferState) with
                     | .error e => .error e
                     | .ok (sawIrref, resTy, stF) =>
                       (match resTy with
                        | none => .error (.unsupported "type oracle: match produced no result type")
                        | some τr =>
                          if sawIrref then .ok (τr, stF)   -- an irrefutable tuple/catch-all arm ⇒ exhaustive
                          -- else: only refutable arms (a nested variant sub-pattern, T1.52 inc-5) with no
                          -- covering catch-all → not provably exhaustive → DECLINE (sound, not a false CDZ0210)
                          else .error (.unsupported "type oracle: tuple match not provably exhaustive (only refutable nested sub-patterns, no catch-all) — declined")))
                  | .record fields =>
                    -- T1.52 inc-2 — MATCH on a RECORD scrutinee: each arm is a `(record (= k p)…)` pattern
                    -- whose field-key set matches the record's (binding each field type) or a catch-all,
                    -- via `recordPatClassify?`. A record has a single shape ⇒ a modeled pattern is
                    -- irrefutable ⇒ exhaustive; an unmodeled pattern declines (never a false reject).
                    let arms := children.extract 2 children.size
                    if arms.size == 0 then .error (.unsupported "type oracle: match with no arms")
                    else (match arms.foldlM (m := Except InferFail)
                        (fun (acc : Bool × Option Ty × InferState) armId =>
                          match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                          | none => .error (.unsupported "type oracle: malformed match arm")
                          | some ac =>
                            (match ac[0]?, ac[1]? with
                             | some patId, some bodyId =>
                               (match recordPatClassify? m fields patId with
                                | none => .error (.unsupported "type oracle: unmodeled record match pattern — declined")
                                | some (irref, binds) =>
                                  (match inferE m (binds.map (fun b => (b.1, ([], b.2))) ++ env) acc.2.2 bodyId with
                                   | .error e => .error e
                                   | .ok (τb, st') =>
                                     (match acc.2.1 with
                                      | none => .ok (acc.1 || irref, some τb, st')
                                      | some τr =>
                                        (match unifyInfer τb τr st' with
                                         | .error e => .error e
                                         | .ok st'' => .ok (acc.1 || irref, some τr, st'')))))
                             | _, _ => .error (.unsupported "type oracle: malformed match arm")))
                        ((false, none, st0) : Bool × Option Ty × InferState) with
                     | .error e => .error e
                     | .ok (sawIrref, resTy, stF) =>
                       (match resTy with
                        | none => .error (.unsupported "type oracle: match produced no result type")
                        | some τr =>
                          if sawIrref then .ok (τr, stF)
                          -- refutable-only record arms (nested variant, inc-5) with no catch-all → decline (sound)
                          else .error (.unsupported "type oracle: record match not provably exhaustive (only refutable nested sub-patterns, no catch-all) — declined")))
                  | .listTy elem =>
                    -- T1.52 inc-3 — MATCH on a LIST scrutinee: arms are `#list(p1…pn)` / `#list(p1…pk (.. r))`
                    -- / catch-all, via `listPatClassify?` (each element binds `elem`, a rest binds `List elem`).
                    -- A list is REFUTABLE (variable length) so exhaustiveness needs an arm covering ALL lengths
                    -- (catch-all / whole-list rest); a match without one is DECLINED (not asserted CDZ0210 — the
                    -- sound first cut). An unmodeled sub-pattern declines. Arm bodies unify.
                    let arms := children.extract 2 children.size
                    if arms.size == 0 then .error (.unsupported "type oracle: match with no arms")
                    else (match arms.foldlM (m := Except InferFail)
                        (fun (acc : Bool × Option Ty × InferState) armId =>
                          match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                          | none => .error (.unsupported "type oracle: malformed match arm")
                          | some ac =>
                            (match ac[0]?, ac[1]? with
                             | some patId, some bodyId =>
                               (match listPatClassify? m elem patId with
                                | none => .error (.unsupported "type oracle: unmodeled list match pattern — declined")
                                | some (coversAll, binds) =>
                                  (match inferE m (binds.map (fun b => (b.1, ([], b.2))) ++ env) acc.2.2 bodyId with
                                   | .error e => .error e
                                   | .ok (τb, st') =>
                                     (match acc.2.1 with
                                      | none => .ok (acc.1 || coversAll, some τb, st')
                                      | some τr =>
                                        (match unifyInfer τb τr st' with
                                         | .error e => .error e
                                         | .ok st'' => .ok (acc.1 || coversAll, some τr, st'')))))
                             | _, _ => .error (.unsupported "type oracle: malformed match arm")))
                        ((false, none, st0) : Bool × Option Ty × InferState) with
                     | .error e => .error e
                     | .ok (sawCoversAll, resTy, stF) =>
                       (match resTy with
                        | none => .error (.unsupported "type oracle: match produced no result type")
                        | some τr =>
                          if sawCoversAll then .ok (τr, stF)
                          else .error (.unsupported "type oracle: list match not provably exhaustive (no catch-all / whole-list rest) — declined")))
                  | _ => .error (.unsupported "type oracle: match scrutinee is not a modeled sum type")))
          else if ((userSumMap m).bind (fun mp => mp.find? (fun e => e.1 == h))).isSome then
            -- T1.25 — USER SUM CONSTRUCTION: `h` is a declared variant `(type T … (h τ) …)` → its type's
            -- structural sum + payload. `(h x)` unifies `x` with the declared payload τ → the sum; a nullary
            -- variant `(h)`/`(h unit)` → the sum. Over-application (children > 2) → `CDZ0203` (single-arity).
            (match (userSumMap m).bind (fun mp => mp.find? (fun e => e.1 == h)) with
             | some (_, (_, sumTy, some τp)) =>
               if children.size > 2 then .error (.illTyped "CDZ0203")
               else (match children[1]? with
                     | some xId => (match inferE m env st xId with
                                    | .ok (τ, st') => (match unifyInfer τ τp st' with
                                                       | .ok st'' => .ok (sumTy, st'')
                                                       | .error e => .error e)
                                    | .error e => .error e)
                     | none => .error (.unsupported "type oracle: malformed user-sum constructor"))
             | some (_, (_, sumTy, none)) =>
               if children.size > 2 then .error (.illTyped "CDZ0203") else .ok (sumTy, st)
             | none => .error (.unsupported "type oracle: user-sum ctor lookup vanished"))
          else
            -- T1.12/T1.18 — APPLICATION `(f a…)` (`ts:36`), the arrow-elim rule: `f` a NAME bound in the env
            -- to a scheme; INSTANTIATE it (fresh vars per use — this is where `let`-polymorphism pays off),
            -- then unify the (curried) instantiated fn type against each argument to a fresh result var,
            -- yielding the codomain. A let-bound `(fn (a) a) : ∀α.α→α` instantiates to a fresh `β→β` at each
            -- call, so it types at several argument types; a MONOMORPHIC (λ/param-bound) `f` instantiates to
            -- itself, so using it at two types clashes → `CDZ0203` (correct — a λ-bound name is monomorphic).
            -- A head not bound in the env (a prelude/builtin) → `Unsupported`. `(f)` (no args) = grouping →
            -- `f`'s type. Applying a non-function type, or an arg-domain clash → `IllTyped CDZ0203`.
            match env.find? (fun e => e.1 == h) with
            | some (_, sch) =>
              let (τf, stInst) := instantiateScheme sch st
              (match (children.extract 1 children.size).foldlM (m := Except InferFail)
                    (fun (acc : Ty × InferState) aid =>
                      match inferE m env acc.2 aid with
                      | .ok (τa, st1) =>
                        let β : Ty := .var st1.next
                        let st2 := { st1 with next := st1.next + 1 }
                        let headTy := applySubst st2.subst acc.1
                        (match unifyInfer headTy (.fn τa β) st2 with
                         | .ok st3 => .ok (applySubst st3.subst β, st3)
                         | .error (.illTyped c) =>
                           -- 🪤 MONOMORPHIZATION (rcdzc, v-cdz-smith probe): rcdzc monomorphizes a fn-PARAM
                           -- per call-site, so a param re-used at conflicting types is ACCEPTED when the
                           -- actual arg is polymorphic (e.g. `(fn (f) (tuple (f 5) (f true)))` applied to
                           -- `(fn (x) x)`) but REJECTED for a concrete arg. Typing the fn in ISOLATION can't
                           -- see the call-site arg, so if the head type STILL carries a free var at the clash
                           -- (a re-used param, not a concrete fn), DECLINE (`Unsupported`) rather than assert
                           -- `CDZ0203` — sound (skip, never a false reject). A CONCRETE-fn head clash (no var)
                           -- is a genuine arg-type error → keep `CDZ0203` (rcdzc rejects it too). Use
                           -- `hasGenVar` (a true polymorphic var), NOT `hasVar`: a `.numVar`-typed head (e.g.
                           -- `(fn (x) (+ x 1)) : numVar→numVar` applied to `#t`) is a GENUINE numeric-vs-Bool
                           -- reject rcdzc also makes — a numVar defaults to Int, it is not a monomorphizable param.
                           if hasGenVar headTy then .error (.unsupported
                             "type oracle: higher-order fn-param applied at conflicting types — monomorphization-dependent, declined")
                           else .error (.illTyped c)
                         | .error e => .error e)
                      | .error e => .error e) (τf, stInst) with
                 | .ok (τres, st') => .ok (τres, st')
                 | .error e => .error e)
            | none => .error (.unsupported
                "type oracle: unmodeled application head / construct (App/Match — prelude heads decline)")
        | none =>
          (match Eval.qualHead? m children with
           | some (q, op) =>
             if q == "List".toUTF8 then
               -- T1.30/31 — total List OPS `(List.<op> …)`: `len (xs:List α) → Int64`; `at (xs:List α)(i:Int)
               -- → (Option α)` (T1.34 fix: indexing is TOTAL-FALLIBLE per collections-and-text.md:134 — it
               -- yields `Some α` in-bounds / `None` OOB, it does NOT trap and is NOT bare `α`; rcdzc types it
               -- `(Option α)`, so bare-`α` was a FALSE-ACCEPT — v-cdz-smith --typegen); `push (xs:List α)(x:α) → List α`;
               -- `prepend (xs:List α)(x:α) → List α` (receiver-first, front-growth twin of push);
               -- `concat (xs:List α)(ys:List α) → List α`; `update (xs:List α)(i:Int)(x:α) → List α`.
               -- Each unifies the list arg with `List β` (fresh β) — a non-list arg is `IllTyped CDZ0203`; an
               -- index/element clash is `CDZ0203`. Any other List op → `Unsupported` (declined, sound).
               (match children[1]? with
                | some xsId =>
                  (match inferE m env st xsId with
                   | .ok (τxs, st1) =>
                     let β : Ty := .var st1.next
                     let st2 := { st1 with next := st1.next + 1 }
                     (match unifyInfer τxs (.listTy β) st2 with
                      | .error e => .error e
                      | .ok st3 =>
                        if op == "len".toUTF8 && children.size == 2 then .ok (.int 64 true, st3)
                        else if op == "at".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some iId => (match inferE m env st3 iId with
                                          | .ok (τi, st4) => (match unifyInfer τi (.numVar st4.next) { st4 with next := st4.next + 1 } with
                                                              | .ok st5 => .ok (optionTy (applySubst st5.subst β), st5)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed List.at"))
                        else if op == "push".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some xId => (match inferE m env st3 xId with
                                          | .ok (τx, st4) => (match unifyInfer τx β st4 with
                                                              | .ok st5 => .ok (.listTy (applySubst st5.subst β), st5)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed List.push"))
                        else if op == "prepend".toUTF8 && children.size == 3 then
                          -- `prepend` types exactly like `push` (list, elem) → List of the elem type.
                          (match children[2]? with
                           | some xId => (match inferE m env st3 xId with
                                          | .ok (τx, st4) => (match unifyInfer τx β st4 with
                                                              | .ok st5 => .ok (.listTy (applySubst st5.subst β), st5)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed List.prepend"))
                        else if op == "concat".toUTF8 && children.size == 3 then
                          -- `concat` (xs ys): both lists share the element type → unify ys with `List β`.
                          (match children[2]? with
                           | some ysId => (match inferE m env st3 ysId with
                                           | .ok (τys, st4) => (match unifyInfer τys (.listTy β) st4 with
                                                                | .ok st5 => .ok (.listTy (applySubst st5.subst β), st5)
                                                                | .error e => .error e)
                                           | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed List.concat"))
                        else if op == "update".toUTF8 && children.size == 4 then
                          -- `update` (xs i x): index numeric, replacement of the element type → List β.
                          -- (A constant OOB index is a provable trap rcdzc REJECTS via CDZ0304 — a value-level
                          -- reject, out of a TYPE oracle's scope; the conformance gate is the authority. If a
                          -- corpus case surfaces the false-accept, guard on a constant index.)
                          (match children[2]?, children[3]? with
                           | some iId, some xId =>
                             (match inferE m env st3 iId with
                              | .ok (τi, st4) =>
                                (match unifyInfer τi (.numVar st4.next) { st4 with next := st4.next + 1 } with
                                 | .ok st5 =>
                                   (match inferE m env st5 xId with
                                    | .ok (τx, st6) => (match unifyInfer τx β st6 with
                                                        | .ok st7 => .ok (.listTy (applySubst st7.subst β), st7)
                                                        | .error e => .error e)
                                    | .error e => .error e)
                                 | .error e => .error e)
                              | .error e => .error e)
                           | _, _ => .error (.unsupported "type oracle: malformed List.update"))
                        else .error (.unsupported "type oracle: unmodeled List op"))
                   | .error e => .error e)
                | none => .error (.unsupported "type oracle: malformed List op (no list arg)"))
             else if q == "Set".toUTF8 then
               -- T1.32 — Set OPS `(Set.<op> …)`: `of (xs:List α) → Set α`; `to-list (s:Set α) → List α`;
               -- `contains (s:Set α)(x:α) → Bool`; `len (s:Set α) → Int64`; `insert`/`remove (s:Set α)(x:α)
               -- → Set α`; `union`/`intersection`/`difference (s:Set α)(t:Set α) → Set α`. Fresh β per call;
               -- a non-set/list arg or an element clash is `IllTyped CDZ0203`. Any other Set op → declined.
               let β : Ty := .var st.next
               let st0 := { st with next := st.next + 1 }
               if op == "of".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some xsId => (match inferE m env st0 xsId with
                                  | .ok (τxs, st1) => (match unifyInfer τxs (.listTy β) st1 with
                                                       | .ok st2 => .ok (.setTy (applySubst st2.subst β), st2)
                                                       | .error e => .error e)
                                  | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Set.of"))
               else if op == "to-list".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some sId => (match inferE m env st0 sId with
                                 | .ok (τs, st1) => (match unifyInfer τs (.setTy β) st1 with
                                                     | .ok st2 =>
                                                       let elemT := applySubst st2.subst β
                                                       -- Set.to-list ORDERS the elements; a set/map-leaf element
                                                       -- has no blessed total order → coded CDZ0203 (19-sets:4340).
                                                       if containsSetOrMap elemT then .error (.illTyped "CDZ0203")
                                                       else .ok (.listTy elemT, st2)
                                                     | .error e => .error e)
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Set.to-list"))
               else if op == "len".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some sId => (match inferE m env st0 sId with
                                 | .ok (τs, st1) => (match unifyInfer τs (.setTy β) st1 with
                                                     | .ok st2 => .ok (.int 64 true, st2)
                                                     | .error e => .error e)
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Set.len"))
               else if op == "contains".toUTF8 && children.size == 3 then
                 (match children[1]?, children[2]? with
                  | some sId, some xId =>
                    (match inferE m env st0 sId with
                     | .ok (τs, st1) => (match unifyInfer τs (.setTy β) st1 with
                                         | .ok st2 => (match inferE m env st2 xId with
                                                       | .ok (τx, st3) => (match unifyInfer τx β st3 with
                                                                           | .ok st4 => .ok (.bool, st4)
                                                                           | .error e => .error e)
                                                       | .error e => .error e)
                                         | .error e => .error e)
                     | .error e => .error e)
                  | _, _ => .error (.unsupported "type oracle: malformed Set.contains"))
               else if (op == "insert".toUTF8 || op == "remove".toUTF8) && children.size == 3 then
                 (match children[1]?, children[2]? with
                  | some sId, some xId =>
                    (match inferE m env st0 sId with
                     | .ok (τs, st1) => (match unifyInfer τs (.setTy β) st1 with
                                         | .ok st2 => (match inferE m env st2 xId with
                                                       | .ok (τx, st3) => (match unifyInfer τx β st3 with
                                                                           | .ok st4 => .ok (.setTy (applySubst st4.subst β), st4)
                                                                           | .error e => .error e)
                                                       | .error e => .error e)
                                         | .error e => .error e)
                     | .error e => .error e)
                  | _, _ => .error (.unsupported "type oracle: malformed Set.insert/remove"))
               else if (op == "union".toUTF8 || op == "intersection".toUTF8 || op == "difference".toUTF8)
                       && children.size == 3 then
                 (match children[1]?, children[2]? with
                  | some sId, some tId =>
                    (match inferE m env st0 sId with
                     | .ok (τs, st1) => (match unifyInfer τs (.setTy β) st1 with
                                         | .ok st2 => (match inferE m env st2 tId with
                                                       | .ok (τt, st3) => (match unifyInfer τt (.setTy β) st3 with
                                                                           | .ok st4 => .ok (.setTy (applySubst st4.subst β), st4)
                                                                           | .error e => .error e)
                                                       | .error e => .error e)
                                         | .error e => .error e)
                     | .error e => .error e)
                  | _, _ => .error (.unsupported "type oracle: malformed Set binary op"))
               else .error (.unsupported "type oracle: unmodeled Set op")
             else if q == "Map".toUTF8 then
               -- T1.33 — Map OPS `(Map.<op> …)` (receiver-first, sigs from prelude.rs map_module): `insert
               -- (Map k v) k v → Map k v`; `lookup (Map k v) k → (Option v)`; `remove (Map k v) k → Map k v`;
               -- `merge (Map k v)(Map k v) → Map k v`; `len (Map k v) → Int64`; `to-list (Map k v) →
               -- (List (Tuple k v))`; `swap (Map k v) k v → (Tuple (Option v)(Map k v))`; `take (Map k v) k →
               -- (Tuple (Option v)(Map k v))`. Fresh k,v per call; a wrong-shape arg/clash → CDZ0203. `empty`
               -- (polymorphic undetermined) + any other member → declined (sound; `empty` is a later increment).
               -- Every modeled op takes the receiver map as children[1]; infer + unify it with `(Map K V)`
               -- ONCE, then dispatch on `op` for the remaining args (explicit nested matches — the proven
               -- pattern; no higher-order local helpers, which defeat the compiler's `inferE` specializer).
               let K : Ty := .var st.next
               let V : Ty := .var (st.next + 1)
               let st0 := { st with next := st.next + 2 }
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed Map op (no map arg — e.g. Map.empty, declined)")
                | some mId =>
                  (match inferE m env st0 mId with
                   | .error e => .error e
                   | .ok (τm, st1) =>
                     (match unifyInfer τm (.mapTy K V) st1 with
                      | .error e => .error e
                      | .ok st2 =>
                        if op == "len".toUTF8 && children.size == 2 then .ok (.int 64 true, st2)
                        else if op == "to-list".toUTF8 && children.size == 2 then
                          -- orders by KEY (values ride along, 19-sets:4354); a set/map-leaf KEY has no
                          -- blessed total order → coded CDZ0203. The value type is unconstrained.
                          let kT := applySubst st2.subst K
                          if containsSetOrMap kT then .error (.illTyped "CDZ0203")
                          else .ok (.listTy (.tuple [kT, applySubst st2.subst V]), st2)
                        else if op == "lookup".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some kId => (match inferE m env st2 kId with
                                          | .ok (τk, st3) => (match unifyInfer τk K st3 with
                                                              | .ok st4 => .ok (optionTy (applySubst st4.subst V), st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Map.lookup"))
                        else if op == "remove".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some kId => (match inferE m env st2 kId with
                                          | .ok (τk, st3) => (match unifyInfer τk K st3 with
                                                              | .ok st4 => .ok (.mapTy (applySubst st4.subst K) (applySubst st4.subst V), st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Map.remove"))
                        else if op == "take".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some kId => (match inferE m env st2 kId with
                                          | .ok (τk, st3) => (match unifyInfer τk K st3 with
                                                              | .ok st4 => .ok (.tuple [optionTy (applySubst st4.subst V), .mapTy (applySubst st4.subst K) (applySubst st4.subst V)], st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Map.take"))
                        else if op == "merge".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some m2Id => (match inferE m env st2 m2Id with
                                           | .ok (τm2, st3) => (match unifyInfer τm2 (.mapTy K V) st3 with
                                                                | .ok st4 => .ok (.mapTy (applySubst st4.subst K) (applySubst st4.subst V), st4)
                                                                | .error e => .error e)
                                           | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Map.merge"))
                        else if op == "insert".toUTF8 && children.size == 4 then
                          (match children[2]?, children[3]? with
                           | some kId, some vId =>
                             (match inferE m env st2 kId with
                              | .ok (τk, st3) => (match unifyInfer τk K st3 with
                                                  | .ok st4 => (match inferE m env st4 vId with
                                                                | .ok (τv, st5) => (match unifyInfer τv V st5 with
                                                                                    | .ok st6 => .ok (.mapTy (applySubst st6.subst K) (applySubst st6.subst V), st6)
                                                                                    | .error e => .error e)
                                                                | .error e => .error e)
                                                  | .error e => .error e)
                              | .error e => .error e)
                           | _, _ => .error (.unsupported "type oracle: malformed Map.insert"))
                        else if op == "swap".toUTF8 && children.size == 4 then
                          (match children[2]?, children[3]? with
                           | some kId, some vId =>
                             (match inferE m env st2 kId with
                              | .ok (τk, st3) => (match unifyInfer τk K st3 with
                                                  | .ok st4 => (match inferE m env st4 vId with
                                                                | .ok (τv, st5) => (match unifyInfer τv V st5 with
                                                                                    | .ok st6 => .ok (.tuple [optionTy (applySubst st6.subst V), .mapTy (applySubst st6.subst K) (applySubst st6.subst V)], st6)
                                                                                    | .error e => .error e)
                                                                | .error e => .error e)
                                                  | .error e => .error e)
                              | .error e => .error e)
                           | _, _ => .error (.unsupported "type oracle: malformed Map.swap"))
                        else .error (.unsupported "type oracle: unmodeled Map op"))))
             else if q == "String".toUTF8 then
               -- T1.38 — String OPS `(String.<op> …)` (receiver-first, sigs from prelude.rs string_module):
               -- `scalar-len`/`byte-len (s) → Int64`; `concat (s)(t) → String`; `at (s)(i) → (Option String)`
               -- and `scalar-at (s)(i) → (Option Char)` and `slice (s)(a)(b) → (Option String)` are all
               -- TOTAL-FALLIBLE (Option, never trap — collections-and-text.md, same as List.at). The receiver
               -- (children[1]) must be a String; a non-String / non-numeric index is CDZ0203. Other member → declined.
               -- `to-bytes (s:String) → Bytes` and `from-bytes (b:Bytes) → (Option String)` (T1.39, fallible
               -- UTF-8 decode) bridge String↔Bytes; from-bytes takes a BYTES receiver so it is handled FIRST,
               -- before the String-receiver unification below.
               if op == "from-bytes".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some bId => (match inferE m env st bId with
                                 | .ok (τb, st1) => (match unifyInfer τb .bytes st1 with
                                                     | .ok st2 => .ok (optionTy .string, st2)
                                                     | .error e => .error e)
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed String.from-bytes"))
               else
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed String op (no receiver)")
                | some sId =>
                  (match inferE m env st sId with
                   | .error e => .error e
                   | .ok (τs, st1) =>
                     (match unifyInfer τs .string st1 with
                      | .error e => .error e
                      | .ok st2 =>
                        if op == "scalar-len".toUTF8 && children.size == 2 then .ok (.int 64 true, st2)
                        else if op == "byte-len".toUTF8 && children.size == 2 then .ok (.int 64 true, st2)
                        else if op == "concat".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some tId => (match inferE m env st2 tId with
                                          | .ok (τt, st3) => (match unifyInfer τt .string st3 with
                                                              | .ok st4 => .ok (.string, st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed String.concat"))
                        else if (op == "at".toUTF8 || op == "scalar-at".toUTF8) && children.size == 3 then
                          (match children[2]? with
                           | some iId => (match inferE m env st2 iId with
                                          | .ok (τi, st3) => (match unifyInfer τi (.numVar st3.next) { st3 with next := st3.next + 1 } with
                                                              | .ok st4 => .ok (optionTy (if op == "at".toUTF8 then .string else .char), st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed String.at/scalar-at"))
                        else if op == "slice".toUTF8 && children.size == 4 then
                          (match children[2]?, children[3]? with
                           | some aId, some bId =>
                             (match inferE m env st2 aId with
                              | .ok (τa, st3) =>
                                (match unifyInfer τa (.numVar st3.next) { st3 with next := st3.next + 1 } with
                                 | .ok st4 =>
                                   (match inferE m env st4 bId with
                                    | .ok (τb, st5) => (match unifyInfer τb (.numVar st5.next) { st5 with next := st5.next + 1 } with
                                                        | .ok st6 => .ok (optionTy .string, st6)
                                                        | .error e => .error e)
                                    | .error e => .error e)
                                 | .error e => .error e)
                              | .error e => .error e)
                           | _, _ => .error (.unsupported "type oracle: malformed String.slice"))
                        else if op == "to-bytes".toUTF8 && children.size == 2 then .ok (.bytes, st2)
                        else .error (.unsupported "type oracle: unmodeled String op"))))
             else if q == "Bytes".toUTF8 then
               -- T1.39 — Bytes OPS `(Bytes.<op> …)` (sigs from prelude.rs bytes_module): `of (xs:List UInt8)
               -- → Bytes` (TOTAL — a UInt8 element is in range by TYPE); `len (b) → Int64`; `at (b)(i) →
               -- (Option Int64)` and `slice (b)(a)(z) → (Option Bytes)` are TOTAL-FALLIBLE; `concat (b)(c) →
               -- Bytes`; `compact (b) → Bytes`. `of`'s receiver is a LIST (not Bytes) so it is handled first.
               if op == "of".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some xsId => (match inferE m env st xsId with
                                  | .ok (τxs, st1) => (match unifyInfer τxs (.listTy (.int 8 false)) st1 with
                                                       | .ok st2 => .ok (.bytes, st2)
                                                       | .error e => .error e)
                                  | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Bytes.of"))
               else
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed Bytes op (no receiver)")
                | some bId =>
                  (match inferE m env st bId with
                   | .error e => .error e
                   | .ok (τb, st1) =>
                     (match unifyInfer τb .bytes st1 with
                      | .error e => .error e
                      | .ok st2 =>
                        if op == "len".toUTF8 && children.size == 2 then .ok (.int 64 true, st2)
                        else if op == "compact".toUTF8 && children.size == 2 then .ok (.bytes, st2)
                        else if op == "at".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some iId => (match inferE m env st2 iId with
                                          | .ok (τi, st3) => (match unifyInfer τi (.numVar st3.next) { st3 with next := st3.next + 1 } with
                                                              | .ok st4 => .ok (optionTy (.int 64 true), st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Bytes.at"))
                        else if op == "concat".toUTF8 && children.size == 3 then
                          (match children[2]? with
                           | some cId => (match inferE m env st2 cId with
                                          | .ok (τc, st3) => (match unifyInfer τc .bytes st3 with
                                                              | .ok st4 => .ok (.bytes, st4)
                                                              | .error e => .error e)
                                          | .error e => .error e)
                           | none => .error (.unsupported "type oracle: malformed Bytes.concat"))
                        else if op == "slice".toUTF8 && children.size == 4 then
                          (match children[2]?, children[3]? with
                           | some aId, some zId =>
                             (match inferE m env st2 aId with
                              | .ok (τa, st3) =>
                                (match unifyInfer τa (.numVar st3.next) { st3 with next := st3.next + 1 } with
                                 | .ok st4 =>
                                   (match inferE m env st4 zId with
                                    | .ok (τz, st5) => (match unifyInfer τz (.numVar st5.next) { st5 with next := st5.next + 1 } with
                                                        | .ok st6 => .ok (optionTy .bytes, st6)
                                                        | .error e => .error e)
                                    | .error e => .error e)
                                 | .error e => .error e)
                              | .error e => .error e)
                           | _, _ => .error (.unsupported "type oracle: malformed Bytes.slice"))
                        else .error (.unsupported "type oracle: unmodeled Bytes op"))))
             else if q == "Float64".toUTF8 || q == "Float32".toUTF8 then
               -- T1.42 — width-namespaced APPLIED float OPS `(Float64.<op> …)` / `(Float32.<op> …)` (sigs from
               -- prelude.rs float_module_record): `neg (Float w) → (Float w)`; `of-int (Int a) → (Float w)`
               -- (int→float, total); `of (Float a) → (Float w)` (width convert, total). `w` = this module's
               -- width. (`nan` is the unapplied constant, in the projection rule.) Other member → declined.
               let w : Nat := if q == "Float64".toUTF8 then 64 else 32
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed float op (no arg)")
                | some aId =>
                  (match inferE m env st aId with
                   | .error e => .error e
                   | .ok (τa, st1) =>
                     if op == "neg".toUTF8 && children.size == 2 then
                       (match unifyInfer τa (.float w) st1 with | .ok st2 => .ok (.float w, st2) | .error e => .error e)
                     else if op == "of-int".toUTF8 && children.size == 2 then
                       -- source is any int width → unify with a fresh numVar (accepts int/numVar; a non-int → CDZ0203).
                       (match unifyInfer τa (.numVar st1.next) { st1 with next := st1.next + 1 } with
                        | .ok st2 => .ok (.float w, st2) | .error e => .error e)
                     else if op == "of".toUTF8 && children.size == 2 then
                       -- source is any float width → unify with a fresh floatVar (a non-float → CDZ0203).
                       (match unifyInfer τa (.floatVar st1.next) { st1 with next := st1.next + 1 } with
                        | .ok st2 => .ok (.float w, st2) | .error e => .error e)
                     else .error (.unsupported "type oracle: unmodeled float op")))
             else if ((intWidthName? q).isSome) then
               -- T1.43 — width-namespaced INTEGER-MODULE ops `(Int64.<op> …)` / `(UInt8.<op> …)` (sigs from
               -- prelude.rs int_module_record). `w`/`sg` = this module's width/sign. `of`/`wrap : (Int a) → T`
               -- (checked/truncating convert; both TOTAL type-wise — an out-of-range `of` is a runtime TRAP,
               -- a value-level concern, not a type error). `wrapping-{add,sub,mul} : T → T → T`.
               -- `checked-{add,sub,mul} : T → T → (Option T)`. (min/max are the unapplied constants above.)
               (match intWidthName? q with
                | none => .error (.unsupported "type oracle: int-module op")  -- unreachable (guarded)
                | some (w, sg) =>
                  let T : Ty := .int w sg
                  if (op == "of".toUTF8 || op == "wrap".toUTF8) && children.size == 2 then
                    -- source is ANY int width → unify the arg with a fresh numVar; result = THIS width.
                    (match children[1]? with
                     | some aId => (match inferE m env st aId with
                                    | .ok (τa, st1) => (match unifyInfer τa (.numVar st1.next) { st1 with next := st1.next + 1 } with
                                                        | .ok st2 => .ok (T, st2)
                                                        | .error e => .error e)
                                    | .error e => .error e)
                     | none => .error (.unsupported "type oracle: malformed int-module of/wrap"))
                  else if (op == "wrapping-add".toUTF8 || op == "wrapping-sub".toUTF8 || op == "wrapping-mul".toUTF8
                           || op == "checked-add".toUTF8 || op == "checked-sub".toUTF8 || op == "checked-mul".toUTF8)
                          && (children.size == 2 || children.size == 3) then
                    -- T → T → (T | Option T): both operands are THIS width; wrapping → T, checked → (Option T).
                    -- T1.51 — PARTIAL APPLICATION (#8313 currying): applied to ONE arg (children.size == 2) yields
                    -- a CLOSURE `T → R` of the remaining param (rcdzc curries a built-in op to a closure). Full
                    -- application (size 3) → `R` as before. `((Int64.wrapping-add 3) 4)` / a bound partial then
                    -- apply the closure via the ordinary fn-application rule.
                    let isChecked := op == "checked-add".toUTF8 || op == "checked-sub".toUTF8 || op == "checked-mul".toUTF8
                    let R : Ty := if isChecked then optionTy T else T
                    if children.size == 2 then
                      -- PARTIAL: unify the one supplied arg with T, result is the closure `T → R`.
                      (match children[1]? with
                       | some aId => (match inferE m env st aId with
                                      | .ok (τa, st1) => (match unifyInfer τa T st1 with
                                                          | .ok st2 => .ok (.fn T R, st2)
                                                          | .error e => .error e)
                                      | .error e => .error e)
                       | none => .error (.unsupported "type oracle: malformed int-module partial op"))
                    else
                      (match children[1]?, children[2]? with
                       | some aId, some bId =>
                         (match inferE m env st aId with
                          | .ok (τa, st1) =>
                            (match unifyInfer τa T st1 with
                             | .ok st2 =>
                               (match inferE m env st2 bId with
                                | .ok (τb, st3) => (match unifyInfer τb T st3 with
                                                    | .ok st4 => .ok (R, st4)
                                                    | .error e => .error e)
                                | .error e => .error e)
                             | .error e => .error e)
                          | .error e => .error e)
                       | _, _ => .error (.unsupported "type oracle: malformed int-module binary op"))
                  else .error (.unsupported "type oracle: unmodeled int-module op"))
             else if q == "Rational".toUTF8 then
               -- T1.44 — Rational OPS `(Rational.<op> …)` (sigs from prelude.rs rational_module): `of (Int a)
               -- (Int b) → Rational`; `of-int (Int a) → Rational`; `value`/`neg (Rational) → Rational`;
               -- `truncate (Rational) → Int64` (integer part toward zero). `numerator`/`denominator` → BigInt
               -- (unmodeled) → declined. A wrong-shape arg → CDZ0203.
               if op == "of".toUTF8 && children.size == 3 then
                 -- (Int a)(Int b) → Rational: both args any int (unify with fresh numVars).
                 (match children[1]?, children[2]? with
                  | some nId, some dId =>
                    (match inferE m env st nId with
                     | .ok (τn, st1) =>
                       (match unifyInfer τn (.numVar st1.next) { st1 with next := st1.next + 1 } with
                        | .ok st2 =>
                          (match inferE m env st2 dId with
                           | .ok (τd, st3) => (match unifyInfer τd (.numVar st3.next) { st3 with next := st3.next + 1 } with
                                               | .ok st4 => .ok (.rational, st4)
                                               | .error e => .error e)
                           | .error e => .error e)
                        | .error e => .error e)
                     | .error e => .error e)
                  | _, _ => .error (.unsupported "type oracle: malformed Rational.of"))
               else if op == "of-int".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some aId => (match inferE m env st aId with
                                 | .ok (τa, st1) => (match unifyInfer τa (.numVar st1.next) { st1 with next := st1.next + 1 } with
                                                     | .ok st2 => .ok (.rational, st2)
                                                     | .error e => .error e)
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Rational.of-int"))
               else if (op == "value".toUTF8 || op == "neg".toUTF8 || op == "truncate".toUTF8
                        || op == "numerator".toUTF8 || op == "denominator".toUTF8) && children.size == 2 then
                 -- receiver is a Rational; value/neg → Rational, truncate → Int64, numerator/denominator →
                 -- BigInt (either component can exceed i64 — T1.46 modeled BigInt).
                 (match children[1]? with
                  | some rId => (match inferE m env st rId with
                                 | .ok (τr, st1) => (match unifyInfer τr .rational st1 with
                                                     | .ok st2 =>
                                                       let res : Ty :=
                                                         if op == "truncate".toUTF8 then .int 64 true
                                                         else if op == "numerator".toUTF8 || op == "denominator".toUTF8 then .bigint
                                                         else .rational
                                                       .ok (res, st2)
                                                     | .error e => .error e)
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Rational unary op"))
               else .error (.unsupported "type oracle: unmodeled Rational op")
             else if q == "BigInt".toUTF8 then
               -- T1.46 — BigInt OPS `(BigInt.<op> …)` (sigs from prelude.rs bigint_module): `of (Int a) →
               -- BigInt` (exact widening, total); `neg (BigInt) → BigInt`. A non-int/non-bigint arg → CDZ0203.
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed BigInt op (no arg)")
                | some aId =>
                  (match inferE m env st aId with
                   | .error e => .error e
                   | .ok (τa, st1) =>
                     if op == "of".toUTF8 && children.size == 2 then
                       -- source is any fixed-width int → unify with a fresh numVar.
                       (match unifyInfer τa (.numVar st1.next) { st1 with next := st1.next + 1 } with
                        | .ok st2 => .ok (.bigint, st2) | .error e => .error e)
                     else if op == "neg".toUTF8 && children.size == 2 then
                       (match unifyInfer τa .bigint st1 with | .ok st2 => .ok (.bigint, st2) | .error e => .error e)
                     else .error (.unsupported "type oracle: unmodeled BigInt op")))
             else if q == "Tuple".toUTF8 then
               -- T1.47 — Tuple OPS `(Tuple.<op> …)` (sigs from resolved.rs Prim::Tuple*): `size (Tuple e…) →
               -- Int64` (arity); `concat`/`cat (Tuple a…)(Tuple b…) → (Tuple a… b…)` (element-list append).
               -- Operand must resolve to a concrete `.tuple`; a non-tuple → CDZ0203, an unresolved var → declined.
               if op == "size".toUTF8 && children.size == 2 then
                 (match children[1]? with
                  | some tId => (match inferE m env st tId with
                                 | .ok (τt, st1) => (match applySubst st1.subst τt with
                                                     | .tuple _ => .ok (.int 64 true, st1)
                                                     | .never => .ok (.int 64 true, st1)
                                                     | .var _ => .error (.unsupported "type oracle: Tuple.size on an unresolved operand")
                                                     | _ => .error (.illTyped "CDZ0203"))
                                 | .error e => .error e)
                  | none => .error (.unsupported "type oracle: malformed Tuple.size"))
               else if (op == "concat".toUTF8 || op == "cat".toUTF8) && children.size == 3 then
                 (match children[1]?, children[2]? with
                  | some aId, some bId =>
                    (match inferE m env st aId with
                     | .ok (τa, st1) =>
                       (match inferE m env st1 bId with
                        | .ok (τb, st2) =>
                          (match applySubst st2.subst τa, applySubst st2.subst τb with
                           | .tuple xs, .tuple ys => .ok (.tuple (xs ++ ys), st2)
                           | .tuple _, _ | _, .tuple _ => .error (.unsupported "type oracle: Tuple.concat with an unresolved/non-tuple operand")
                           | _, _ => .error (.illTyped "CDZ0203"))
                        | .error e => .error e)
                     | .error e => .error e)
                  | _, _ => .error (.unsupported "type oracle: malformed Tuple.concat"))
               else .error (.unsupported "type oracle: unmodeled Tuple op")
             else if q == "Record".toUTF8 then
               -- T1.48 — Record ROW ops `(Record.with r #k v)` / `(Record.extend r #k v)` (sigs from
               -- compute.rs lower_record_insert): the field key is a `#symbol` (`.sym` leaf); the result is
               -- a NEW record type (row update yields a new value — type-system.md). `with` REPLACES a
               -- PRESENT field (its type becomes typeof(v); an ABSENT field is CDZ0212); `extend` ADDS an
               -- ABSENT field (a PRESENT field is CDZ0211). A non-record base → CDZ0203; unresolved →
               -- declined. (merge/without/project/pop are label-list / disjoint / tuple-shaped — later.)
               if (op == "with".toUTF8 || op == "extend".toUTF8) && children.size == 4 then
                 (match children[1]?, children[2]?, children[3]? with
                  | some rId, some kId, some vId =>
                    (match symOf? m kId with
                     | none => .error (.unsupported "type oracle: Record.with/extend key is not a #symbol")
                     | some k =>
                       (match inferE m env st rId with
                        | .error e => .error e
                        | .ok (τr, st1) =>
                          (match applySubst st1.subst τr with
                           | .record fields =>
                             (match inferE m env st1 vId with
                              | .ok (τv, st2) =>
                                let vt := applySubst st2.subst τv
                                let present := fields.any (fun f => Eval.cmpBytes f.1 k == .eq)
                                -- `with` replaces a PRESENT field (absent → CDZ0212); `extend` ADDS an
                                -- ABSENT field (present → CDZ0211). Result is a NEW record type.
                                if op == "with".toUTF8 then
                                  if present then .ok (.record (fields.map (fun f => if Eval.cmpBytes f.1 k == .eq then (k, vt) else f)), st2)
                                  else .error (.illTyped "CDZ0212")
                                else if present then .error (.illTyped "CDZ0211")
                                else
                                  let extended := ((fields ++ [(k, vt)]).toArray.qsort (fun a b => Eval.cmpBytes a.1 b.1 == .lt)).toList
                                  .ok (.record extended, st2)
                              | .error e => .error e)
                           | .var _ => .error (.unsupported "type oracle: Record.with/extend on an unresolved record")
                           | _ => .error (.illTyped "CDZ0203"))))
                  | _, _, _ => .error (.unsupported "type oracle: malformed Record.with/extend"))
               else if op == "merge".toUTF8 && children.size == 3 then
                 -- `merge r s` → the UNION of two records. DISJOINT-only (compute.rs): an overlapping key is
                 -- CDZ0211 (the explicit merge is not last-writer-wins — that is the `#record((..r))` spread).
                 (match children[1]?, children[2]? with
                  | some rId, some sId => do
                    let (τr, st1) ← inferE m env st rId
                    let (τs, st2) ← inferE m env st1 sId
                    match applySubst st2.subst τr, applySubst st2.subst τs with
                    | .record fr, .record fs =>
                      if fr.any (fun f => fs.any (fun g => Eval.cmpBytes f.1 g.1 == .eq)) then .error (.illTyped "CDZ0211")
                      else .ok (.record ((fr ++ fs).toArray.qsort (fun a b => Eval.cmpBytes a.1 b.1 == .lt)).toList, st2)
                    | _, _ => .error (.illTyped "CDZ0203")
                  | _, _ => .error (.unsupported "type oracle: malformed Record.merge"))
               else if op == "pop".toUTF8 && children.size == 3 then
                 -- `pop r #k` → `(tuple (. r k) (r without k))`: the popped field's type paired with the record
                 -- minus k. An ABSENT key is CDZ0212 (the field access fails).
                 (match children[1]?, children[2]? with
                  | some rId, some kId =>
                    (match symOf? m kId with
                     | none => .error (.unsupported "type oracle: Record.pop key is not a #symbol")
                     | some k => do
                       let (τr, st1) ← inferE m env st rId
                       match applySubst st1.subst τr with
                       | .record fields =>
                         (match fields.find? (fun f => Eval.cmpBytes f.1 k == .eq) with
                          | some kv => .ok (.tuple [kv.2, .record (fields.filter (fun f => Eval.cmpBytes f.1 k != .eq))], st1)
                          | none => .error (.illTyped "CDZ0212"))
                       | _ => .error (.illTyped "CDZ0203"))
                  | _, _ => .error (.unsupported "type oracle: malformed Record.pop"))
               else if op == "project".toUTF8 && children.size == 3 then
                 -- `project r (a c)` → narrow r to EXACTLY the named labels (each carrying r's type for it).
                 -- The 2nd operand is a LITERAL label LIST (bare names, not a value). A label ABSENT from r is
                 -- CDZ0212; a DUPLICATE label is CDZ0201. A non-record / unresolved operand or a malformed
                 -- label list → declined (safe abstain — rcdzc's non-record path yields `Any`, not a clear
                 -- reject, so the oracle stays out of that fragment).
                 (match children[1]?, children[2]? with
                  | some rId, some lId =>
                    (match labelsOf? m lId with
                     | none => .error (.unsupported "type oracle: Record.project labels are not a bare-name list")
                     | some labels => do
                       let (τr, st1) ← inferE m env st rId
                       match applySubst st1.subst τr with
                       | .record fields =>
                         if hasDupBytes labels then .error (.illTyped "CDZ0201")
                         else if labels.any (fun l => !fields.any (fun f => Eval.cmpBytes f.1 l == .eq)) then .error (.illTyped "CDZ0212")
                         else
                           let kept := labels.filterMap (fun l => (fields.find? (fun f => Eval.cmpBytes f.1 l == .eq)).map (fun f => (l, f.2)))
                           .ok (.record ((kept.toArray.qsort (fun a b => Eval.cmpBytes a.1 b.1 == .lt)).toList), st1)
                       | .var _ => .error (.unsupported "type oracle: Record.project on an unresolved record")
                       | _ => .error (.unsupported "type oracle: Record.project on a non-record operand"))
                  | _, _ => .error (.unsupported "type oracle: malformed Record.project"))
               else if op == "without".toUTF8 && children.size == 3 then
                 -- `without r (b)` → r MINUS the named labels (the complement of `project`). Same literal label
                 -- list, same faults: absent label → CDZ0212, duplicate → CDZ0201. Non-record / unresolved /
                 -- malformed-list → declined.
                 (match children[1]?, children[2]? with
                  | some rId, some lId =>
                    (match labelsOf? m lId with
                     | none => .error (.unsupported "type oracle: Record.without labels are not a bare-name list")
                     | some labels => do
                       let (τr, st1) ← inferE m env st rId
                       match applySubst st1.subst τr with
                       | .record fields =>
                         if hasDupBytes labels then .error (.illTyped "CDZ0201")
                         else if labels.any (fun l => !fields.any (fun f => Eval.cmpBytes f.1 l == .eq)) then .error (.illTyped "CDZ0212")
                         else .ok (.record (fields.filter (fun f => !labels.any (fun l => Eval.cmpBytes f.1 l == .eq))), st1)
                       | .var _ => .error (.unsupported "type oracle: Record.without on an unresolved record")
                       | _ => .error (.unsupported "type oracle: Record.without on a non-record operand"))
                  | _, _ => .error (.unsupported "type oracle: malformed Record.without"))
               else .error (.unsupported "type oracle: unmodeled Record op")
             else if q == "Char".toUTF8 then
               -- T1.45 — Char OPS `(Char.<op> …)` (sigs from prelude.rs char_module): `to-int (Char) → Int64`
               -- (total scalar-value read); `from-int (Int64) → (Option Char)` (fallible int→char — an out-of-
               -- range/surrogate code point → None). from-int's arg is an INT, not a Char.
               (match children[1]? with
                | none => .error (.unsupported "type oracle: malformed Char op (no arg)")
                | some aId =>
                  (match inferE m env st aId with
                   | .error e => .error e
                   | .ok (τa, st1) =>
                     if op == "to-int".toUTF8 && children.size == 2 then
                       (match unifyInfer τa .char st1 with | .ok st2 => .ok (.int 64 true, st2) | .error e => .error e)
                     else if op == "from-int".toUTF8 && children.size == 2 then
                       (match unifyInfer τa (.numVar st1.next) { st1 with next := st1.next + 1 } with
                        | .ok st2 => .ok (optionTy .char, st2) | .error e => .error e)
                     else .error (.unsupported "type oracle: unmodeled Char op")))
             else
               -- T1.27 — APPLIED QUALIFIED user ctor `((. Q M) arg)` = `(Q.M arg)`: `qualHead?` reads the
               -- `(. Q M)` head; if `M` is a variant whose declaring type name is `Q`, construct its sum
               -- (unify the arg with M's payload; nullary M takes an optional unit arg). Over-apply → CDZ0203.
               (match (userSumMap m).bind (fun mp =>
                        (mp.find? (fun e => e.1 == op)).bind (fun ent => if ent.2.1 == q then some ent else none)) with
                | some (_, (_, sumTy, some τp)) =>
                  if children.size > 2 then .error (.illTyped "CDZ0203")
                  else (match children[1]? with
                        | some xId => (match inferE m env st xId with
                                       | .ok (τ, st') => (match unifyInfer τ τp st' with
                                                          | .ok st'' => .ok (sumTy, st'')
                                                          | .error e => .error e)
                                       | .error e => .error e)
                        | none => .error (.unsupported "type oracle: malformed qualified user-sum constructor"))
                | some (_, (_, sumTy, none)) =>
                  if children.size > 2 then .error (.illTyped "CDZ0203") else .ok (sumTy, st)
                | none => .error (.unsupported "type oracle: non-name-headed construct not yet modeled"))
           | none =>
             -- T1.51 — APPLY A NON-NAME EXPRESSION HEAD `((<expr>) args…)` where `<expr>` infers to a `.fn`
             -- (e.g. a built-in PARTIAL application: `((Int64.wrapping-add 3) 4)`). Infer the head; if it's a
             -- function type, fold-apply the args (unify each arg with the domain, take the codomain), mirroring
             -- the name-headed fn-application rule. A non-`.fn` head → decline (unmodeled, sound skip).
             (match children[0]? with
              | some hId =>
                (match inferE m env st hId with
                 | .ok (τh, stH) =>
                   (match applySubst stH.subst τh with
                    | .fn _ _ =>
                      ((children.extract 1 children.size).foldlM (m := Except InferFail)
                        (fun (acc : Ty × InferState) aid =>
                          match inferE m env acc.2 aid with
                          | .ok (τa, st1) =>
                            let β : Ty := .var st1.next
                            let st2 := { st1 with next := st1.next + 1 }
                            let headTy := applySubst st2.subst acc.1
                            (match unifyInfer headTy (.fn τa β) st2 with
                             | .ok st3 => .ok (applySubst st3.subst β, st3)
                             | .error e => .error e)
                          | .error e => .error e) (applySubst stH.subst τh, stH))
                    | _ => .error (.unsupported "type oracle: non-name-headed construct not yet modeled"))
                 | .error e => .error e)
              | none => .error (.unsupported "type oracle: non-name-headed construct not yet modeled")))
      | _ => .error (.unsupported "type oracle: node not modeled")

/-- Default any still-unresolved numeric var (an int literal never constrained to a concrete width) to the
model-default `Int64` — the numeric-model default for an unannotated literal at an escape (OQ-G). -/
partial def defaultNumVars : Ty → Ty
  | .numVar _ => .int 64 true
  | .floatVar _ => .float 64                 -- an unconstrained float literal defaults to Float64 (01-literals:329)
  | .fn d c => .fn (defaultNumVars d) (defaultNumVars c)
  | .tuple es => .tuple (es.map defaultNumVars)
  | .listTy e => .listTy (defaultNumVars e)
  | .setTy e => .setTy (defaultNumVars e)
  | .mapTy k v => .mapTy (defaultNumVars k) (defaultNumVars v)
  | .record fs => .record (fs.map (fun f => (f.1, defaultNumVars f.2)))
  | .sum vs => .sum (vs.map (fun v => (v.1, v.2.map defaultNumVars)))
  | t => t

/-- Does the type contain a SUM with an UNDETERMINED (free-var) payload? Such a value at the program's
escape (the `main` result) is the `ts:34` rejection — a bare `None` / `Ok x` whose other side never gets
determined. The oracle DECLINES these (Unsupported) rather than positively judging: modeling the exact
consumed-vs-escaping distinction is subtle, so decline is the sound response (a skip, never a false
reject). Targeted at `.sum`-payload vars only — a polymorphic FN result (a `.fn` with vars) is NOT flagged. -/
partial def hasUndeterminedSum : Ty → Bool
  | .sum vs => vs.any (fun v => match v.2 with | some t => hasVar t | none => false)
  | .fn d c => hasUndeterminedSum d || hasUndeterminedSum c
  | .tuple es => es.any hasUndeterminedSum
  | .listTy e => hasUndeterminedSum e
  | .setTy e => hasUndeterminedSum e
  | .mapTy k v => hasUndeterminedSum k || hasUndeterminedSum v
  | .record fs => fs.any (fun f => hasUndeterminedSum f.2)
  | _ => false

/-- Does the type contain a COLLECTION (`.listTy`/`.setTy`/`.mapTy`) whose element/key/value carries an
UNDETERMINED (free general-var) type — e.g. a bare escaping `Map.empty` (`.mapTy var var`)? Such a value at
the program escape has no resolvable type (rcdzc cannot determine it either), so the oracle DECLINES (a
sound skip, mirroring `hasUndeterminedSum`). Runs AFTER `defaultNumVars`, so `numVar`s are already `Int64`
— only genuine general vars remain. Deliberately NO `.fn` arm: a polymorphic FN result is not flagged
(matches `hasUndeterminedSum`'s intent; a fn's collection cod is resolved at its call site, not at escape). -/
partial def hasUndeterminedCollection : Ty → Bool
  | .listTy e => hasGenVar e || hasUndeterminedCollection e
  | .setTy e => hasGenVar e || hasUndeterminedCollection e
  | .mapTy k v => hasGenVar k || hasGenVar v || hasUndeterminedCollection k || hasUndeterminedCollection v
  | .tuple es => es.any hasUndeterminedCollection
  | .record fs => fs.any (fun f => hasUndeterminedCollection f.2)
  | .sum vs => vs.any (fun v => match v.2 with | some t => hasUndeterminedCollection t | none => false)
  | _ => false

/-- Infer the type of a body node under a value environment `env`: run `inferE` and map its result onto
the verdict algebra. A resolved type (with the final substitution applied, unresolved numeric literals
defaulted to `Int64`) is `WellTyped`; a modeled fault is `IllTyped`; a coverage gap is `Unsupported`. -/
def inferBody (m : Ast.Module) (env : List (ByteArray × Scheme)) (st0 : InferState) (nodeId : Nat) : TypeVerdict :=
  match inferE m env st0 nodeId with
  | .ok (τ, st) =>
    let final := defaultNumVars (applySubst st.subst τ)
    if hasUndeterminedSum final then
      .unsupported "type oracle: undetermined sum result at escape (ts:34) — declined"
    else if hasUndeterminedCollection final then
      .unsupported "type oracle: undetermined collection result at escape (e.g. bare Map.empty) — declined"
    else .wellTyped final
  | .error (.illTyped c) => .illTyped c
  | .error (.unsupported r) => .unsupported r

/-- T1.24 — build the TOP-LEVEL environment by typing EVERY sibling def (value def AND fn-def) in the
root `(do …)`, in order, so `main`'s body may reference them. A value def `(def x e)` binds `x :
GENERALIZE(τ)`; a fn-def `(def (f p…) body)` binds `f` via the Fn logic + monomorphic self-recursion +
generalization (mirrors the do-rule T1.9/21/22). `main` itself is skipped (typed separately). Returns the
env AND the final `InferState`, so `main`'s inference CONTINUES the var counter (no id collision with the
env's free `numVar`s). An ill-typed sibling → `IllTyped` (rcdzc rejects the program too); an unmodeled
sibling → `Unsupported` (sound skip — the whole program declines, never a false positive). -/
partial def topLevelEnv (m : Ast.Module) : Except InferFail (List (ByteArray × Scheme) × InferState) :=
  match m.nodes[m.root]? with
  | some (.list stmtsRaw) =>
    (stmtsRaw.extract 1 stmtsRaw.size).foldlM (m := Except InferFail)
      (fun (acc : List (ByteArray × Scheme) × InferState) sid =>
        match Eval.asDef? m sid with
        | none => .ok acc                                        -- export / pragma / non-def → not a binding
        | some dc =>
          (match dc[1]?, dc[dc.size - 1]? with
           | some targetId, some valId =>
             (match Eval.nameOf? m targetId with
              | some nm =>                                       -- value def `(def x e)`
                if nm == "main".toUTF8 then .ok acc
                else (match inferE m acc.1 acc.2 valId with
                      | .ok (τ, st') => .ok (((nm, generalizeScheme acc.1 st'.subst τ) :: acc.1), st')
                      | .error e => .error e)
              | none =>                                          -- fn-def `(def (f p…) body)`
                (match m.nodes[targetId]? with
                 | some (Ast.Node.list tc) =>
                   (match (tc[0]?).bind (Eval.nameOf? m) with
                    | some fnm =>
                      if fnm == "main".toUTF8 then .ok acc
                      else (match (tc.extract 1 tc.size).foldlM (m := Except InferFail)
                          (fun (pacc : List (ByteArray × Scheme) × List Ty × InferState) pid =>
                            match m.nodes[pid]? with
                            | some (Ast.Node.atom plid) =>
                              (match m.leaves[plid]? with
                               | some (.name pnm) =>
                                 let α : Ty := .var pacc.2.2.next
                                 .ok ((pnm, ([], α)) :: pacc.1, α :: pacc.2.1, { pacc.2.2 with next := pacc.2.2.next + 1 })
                               | _ => .error (.unsupported "type oracle: malformed top-level-fn param"))
                            | some (Ast.Node.list ppc) =>
                              (match ppc[1]?, ppc[2]? with
                               | some pnId, some ptId =>
                                 (match Eval.nameOf? m pnId, parseTy? m ptId with
                                  | some pnm, some pτ => .ok ((pnm, ([], pτ)) :: pacc.1, pτ :: pacc.2.1, pacc.2.2)
                                  | some _, none => .error (.unsupported "type oracle: top-level-fn param unmodeled annotation")
                                  | none, _ => .error (.unsupported "type oracle: top-level-fn param missing name"))
                               | _, _ => .error (.unsupported "type oracle: malformed top-level-fn param spec"))
                            | none => .error (.unsupported "type oracle: malformed top-level-fn param"))
                          (acc.1, [], acc.2) with
                       | .ok (bodyEnv, ptysRev, stP) =>
                         -- T1.24 SOUNDNESS: (a) duplicate param names → decline (rcdzc rejects CDZ0102; a
                         -- positive would be a false-reject). (b) a body ill-typed IN ISOLATION → decline,
                         -- NOT reject: rcdzc monomorphizes params per call site, so a param used at a
                         -- compound/context type (tuple/record projection, int8-from-context) can be
                         -- well-typed at the actual arg though our fresh-var HM rejects it → skip is sound.
                         let pnames := (bodyEnv.take ptysRev.length).map (·.1)
                         if pnames.eraseDups.length != pnames.length then
                           .error (.unsupported "type oracle: duplicate top-level-fn param name — declined (rcdzc CDZ0102)")
                         else if ptysRev.any (fun τ => match τ with | .fn _ _ => true | _ => false) then
                           -- HIGHER-ORDER param (a function-typed param, e.g. `(: g (-> Int8 Int8))`): the
                           -- body's use of it is monomorphized/context-typed per call site (narrow-width
                           -- context propagation, etc.) beyond our fresh-var HM → decline (sound skip).
                           .error (.unsupported "type oracle: higher-order fn-def (function-typed param) — context/monomorphization-dependent, declined")
                         else
                           let ρ : Ty := .var stP.next
                           let recArrow := ptysRev.foldl (fun a pτ => Ty.fn pτ a) ρ
                           let stP1 := { stP with next := stP.next + 1 }
                           (match inferE m ((fnm, ([], recArrow)) :: bodyEnv) stP1 valId with
                            | .ok (bodyτ, stB) =>
                              (match unifyInfer bodyτ ρ stB with
                               | .ok stB2 =>
                                 let arrow := ptysRev.foldl (fun a pτ => Ty.fn pτ a) ρ
                                 .ok (((fnm, generalizeScheme acc.1 stB2.subst arrow) :: acc.1), stB2)
                               | .error _ => .error (.unsupported "type oracle: top-level fn-def body ill-typed in isolation — monomorphization-dependent, declined"))
                            | .error (.illTyped _) => .error (.unsupported "type oracle: top-level fn-def body ill-typed in isolation — monomorphization-dependent, declined")
                            | .error (.unsupported r) => .error (.unsupported r))
                       | .error e => .error e)
                    | none => .error (.unsupported "type oracle: top-level fn-def head not a name"))
                 | _ => .error (.unsupported "type oracle: malformed top-level fn-def target")))
           | _, _ => .error (.unsupported "type oracle: malformed top-level def")))
      ([], {})
  | _ => .ok ([], {})

/-- The type oracle. Vet the whole program (`programModeled?`), type all sibling top-level defs into an env
(`topLevelEnv` — value defs + fn-defs, generalized), then infer `main`'s body under that env (continuing the
var counter). A modeled fault (in a sibling or in `main`) is `IllTyped`; a coverage gap is `Unsupported`. -/
def infer (m : Ast.Module) : TypeVerdict :=
  -- WHOLE-PROGRAM soundness gate (design §5): decline unless every top-level statement is a modeled KIND
  -- (value def / fn-def / export-of-defined / pragma); a `(type …)`/`(effect …)`/import or unbound export
  -- declines. The def TYPING gate is `topLevelEnv` (an ill-typed/unmodeled sibling → IllTyped/Unsupported).
  if !programModeled? m then
    .unsupported "type oracle: program has unmodeled top-level structure (effect decl / import / unbound export) — declined for soundness"
  else if (userSumMap m).isNone then
    -- T1.25: the program has `(type …)` decls that aren't soundly modelable STRUCTURALLY (unmodeled payload,
    -- variant arity > 1, or a nominal-ambiguity variant-name collision) → decline (sound skip).
    .unsupported "type oracle: user sum types not soundly modelable (unmodeled payload / arity>1 / variant-name collision) — declined"
  else match Eval.namedParamsBody? m "main".toUTF8 with
  | none => .unsupported "type oracle: program has no (def (main) …) export"
  | some (specs, bodyId) =>
    if specs.size != 0 then .unsupported "type oracle: parameterized main not yet modeled (T1)"
    else match topLevelEnv m with
    | .error (.illTyped c) => .illTyped c
    | .error (.unsupported r) => .unsupported r
    | .ok (env, st0) => inferBody m env st0 bodyId

/-- Is `code` a rejection the TYPE oracle actually judges (a type-system error, §4), vs a rejection from a
DIFFERENT compile phase the oracle does not model? A `WellTyped` verdict against a NON-type reject is NOT a
false-reject — the program genuinely IS well-typed; rcdzc rejected it for well-formedness (`CDZ0201`
duplicate-export / multi-body / out-of-range-literal), const-eval overflow/div-by-zero (`CDZ0304`), range
(`CDZ0302`), or a pragma (`CDZ0601`), none of which is the type judgment. So those degrade to `skip`
(a coverage gap for a phase outside the oracle's remit), keeping false-reject detection focused on genuine
TYPE over-strictness. -/
def isTypeRejectCode (code : Code) : Bool :=
  code == "CDZ0101" || code == "CDZ0102" || code == "CDZ0202" || code == "CDZ0203" || code == "CDZ0301"
  || code == "CDZ0210" || code == "CDZ0211" || code == "CDZ0212" || code == "CDZ0213" || code == "CDZ0214"

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
    if isTypeRejectCode code then
      .mismatch s!"false-reject: oracle infers well-typed, rcdzc rejected {code}"
    else
      .skip s!"typecheck: rcdzc rejected {code} — a non-type-judgment phase (well-formedness/overflow/range/pragma) outside the type oracle's remit"
  | .wellTyped _, .decline =>
    .mismatch s!"capability-gap: oracle infers well-typed, rcdzc declined (should-work-not-yet-built)"
  | .illTyped code, .accept =>
    .mismatch s!"false-accept: oracle infers ill-typed ({code}), rcdzc accepted (soundness hole)"
  | .illTyped _, .reject _ => .holds   -- both reject; code parity is a T3 refinement
  | .illTyped _, .decline => .holds    -- both reject/decline → agree

/-! ### Verdict-classification witnesses (compiled = checked; the §1.2 table). -/

-- an empty / main-less module → the whole-program gate declines → skip (sound coverage gap)
#guard (match judgeTypecheck (infer { leaves := #[], nodes := #[], root := 0 }) .accept with
        | .skip _ => true | _ => false)
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
-- T1.10 (ascription) + OQ-G: `(: 5 Int64)` — the literal (a numVar) resolves to the annotated width →
-- WellTyped Int64 (no longer deferred, now that int literals are width-polymorphic).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "Int64".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2],
                           .atom 2, .list #[4], .atom 1, .list #[6, 5, 3],
                           .atom 6, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped (.int 64 true))
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
-- (T1.12's `(let ((id (fn (x) x))) (id 5))` → Unsupported guard removed — T1.18 let-generalization now
-- types it WellTyped Int64; the T1.18 #guard above asserts that superseding behavior.)
-- T1.24: a top-level sibling fn-def `(def (f x) x)` is now TYPED (f : ∀α.α→α into the env), so
-- `(do (def (f x) x) (def (main) 42) (export main))` → WellTyped Int64 (main's body is 42). (Was
-- Unsupported pre-T1.24, when any fn-def declined the whole program; a well-typed sibling no longer does.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "f".toUTF8, .name "x".toUTF8,
                            .name "main".toUTF8, .intLit false .dec (ByteArray.mk #[42]), .name "export".toUTF8],
                nodes := #[.atom 2, .atom 3, .list #[0, 1], .atom 1, .atom 3, .list #[3, 2, 4],  -- (def (f x) x)
                           .atom 4, .list #[6], .atom 1, .atom 5, .list #[8, 7, 9],              -- (def (main) 42)
                           .atom 6, .atom 4, .list #[11, 12], .atom 0, .list #[14, 5, 10, 13]],
                root := 15 } == .wellTyped (.int 64 true))
-- SOUNDNESS GATE: an export of an UNBOUND name `(export bad)` → Unsupported (the program isn't fully
-- modeled — rcdzc rejects it CDZ0101, but the oracle soundly declines rather than guessing).
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                                  .intLit false .dec (ByteArray.mk #[42]), .name "export".toUTF8, .name "bad".toUTF8],
                      nodes := #[.atom 2, .list #[0], .atom 1, .atom 3, .list #[2, 1, 3],  -- (def (main) 42)
                                 .atom 4, .atom 5, .list #[5, 6], .atom 0, .list #[8, 4, 7]],  -- (export bad)
                      root := 9 } with | .unsupported _ => true | _ => false)
-- ARITY: `(if #t 1 2 3)` (too many operands) → not the exact `(if c t e)` shape → Unsupported (matches
-- rcdzc's CDZ0201 reject at the accept/reject boundary once the gate declines it, never a false WellTyped).
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "if".toUTF8,
                                  .boolLit true, .intLit false .dec (ByteArray.mk #[1]),
                                  .intLit false .dec (ByteArray.mk #[2]), .intLit false .dec (ByteArray.mk #[3]),
                                  .name "export".toUTF8],
                      nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .atom 7, .list #[0, 1, 2, 3, 4],  -- (if #t 1 2 3)
                                 .atom 2, .list #[6], .atom 1, .list #[8, 7, 5],
                                 .atom 8, .atom 2, .list #[11, 12], .atom 0, .list #[14, 9, 13]],
                      root := 14 } with | .unsupported _ => true | _ => false)
-- T1.13 (record): `(record (= a 1) (= b #t))` → WellTyped record {a:Int, b:Bool} (fields sorted by key).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "a".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "b".toUTF8, .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (= a 1)
                           .atom 4, .atom 7, .atom 8, .list #[4, 5, 6],   -- (= b #t)
                           .atom 3, .list #[8, 3, 7],                     -- (record (= a 1)(= b #t))
                           .atom 2, .list #[10], .atom 1, .list #[12, 11, 9],   -- (def (main) …)
                           .atom 9, .atom 2, .list #[14, 15], .atom 0, .list #[17, 13, 16]],
                root := 18 } == .wellTyped (.record [("a".toUTF8, .int 64 true), ("b".toUTF8, .bool)]))
-- T1.14 (field access): `(. (record (= a 1)(= b #t)) b)` → the field-b type = Bool → WellTyped Bool.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "a".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "b".toUTF8, .boolLit true, .name ".".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (= a 1)
                           .atom 4, .atom 7, .atom 8, .list #[4, 5, 6],   -- (= b #t)
                           .atom 3, .list #[8, 3, 7],                     -- (record …)
                           .atom 9, .atom 7, .list #[10, 9, 11],          -- (. <record> b)
                           .atom 2, .list #[13], .atom 1, .list #[15, 14, 12],  -- (def (main) …)
                           .atom 10, .atom 2, .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                root := 21 } == .wellTyped .bool)
-- T1.13 (record): `(record (= a 1) (= a 2))` — a DUPLICATE field name → IllTyped CDZ0211.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "a".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2],   -- (= a 1)
                           .atom 4, .atom 5, .atom 7, .list #[4, 5, 6],   -- (= a 2)
                           .atom 3, .list #[8, 3, 7],                     -- (record (= a 1)(= a 2))
                           .atom 2, .list #[10], .atom 1, .list #[12, 11, 9],
                           .atom 8, .atom 2, .list #[14, 15], .atom 0, .list #[17, 13, 16]],
                root := 18 } == .illTyped "CDZ0211")
-- T1.15 (sum): `(Some 5)` → WellTyped Option Int (determined payload).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "Some".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .list #[0, 1], .atom 2, .list #[3], .atom 1, .list #[5, 4, 2],
                           .atom 5, .atom 2, .list #[7, 8], .atom 0, .list #[10, 6, 9]],
                root := 11 } == .wellTyped (optionTy (.int 64 true)))
-- T1.15 (sum ARITY): over-applying single-arity `Some` — `(Some 5 6)` — is a type error CDZ0203
-- (corpus 09-functions/0208-0209), NOT a silent drop of the surplus arg to `(Some 5)`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "Some".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8,
                            .intLit false .dec (ByteArray.mk #[6])],
                nodes := #[.atom 3, .atom 4, .atom 6, .list #[0, 1, 2], .atom 2, .list #[4],
                           .atom 1, .list #[6, 5, 3], .atom 5, .atom 2, .list #[8, 9], .atom 0,
                           .list #[11, 7, 10]],
                root := 12 } == .illTyped "CDZ0203")
-- T1.15 (sum): `(Less)` → WellTyped Ordering (nullary built-in ctor).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "Less".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 3, .list #[0], .atom 2, .list #[2], .atom 1, .list #[4, 3, 1],
                           .atom 4, .atom 2, .list #[6, 7], .atom 0, .list #[9, 5, 8]],
                root := 10 } == .wellTyped orderingTy)
-- T1.15 (Esc, ts:34): bare `(None)` at the escape → undetermined Option → Unsupported (NOT a false reject).
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "None".toUTF8,
                                  .name "export".toUTF8],
                      nodes := #[.atom 3, .list #[0], .atom 2, .list #[2], .atom 1, .list #[4, 3, 1],
                                 .atom 4, .atom 2, .list #[6, 7], .atom 0, .list #[9, 5, 8]],
                      root := 10 } with | .unsupported _ => true | _ => false)
-- T1.15 (Esc, ts:34): `(Ok 5)` DETERMINES only the Ok side — the Err side stays a free var, so the Result
-- ESCAPES undetermined → Unsupported (a sound decline, NOT a false reject). Contrast `(Some 5)` above, whose
-- single payload IS determined → WellTyped. Pins the asymmetry + the Ok construction→escape path.
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "Ok".toUTF8,
                                  .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                      nodes := #[.atom 3, .atom 4, .list #[0, 1], .atom 2, .list #[3], .atom 1,
                                 .list #[5, 4, 2], .atom 5, .atom 2, .list #[7, 8], .atom 0, .list #[10, 6, 9]],
                      root := 11 } with | .unsupported _ => true | _ => false)
-- T1.15 (Esc, ts:34): `(Err #t)` symmetrically determines only the Err side (Ok side free) → escapes
-- undetermined → Unsupported. Pins the Err construction→escape path.
#guard (match infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "Err".toUTF8,
                                  .boolLit true, .name "export".toUTF8],
                      nodes := #[.atom 3, .atom 4, .list #[0, 1], .atom 2, .list #[3], .atom 1,
                                 .list #[5, 4, 2], .atom 5, .atom 2, .list #[7, 8], .atom 0, .list #[10, 6, 9]],
                      root := 11 } with | .unsupported _ => true | _ => false)
-- T1.16 (Mat): exhaustive Option match `(match (Some 5) ((Some x) x) ((None _) 0))` → WellTyped Int64
-- (Some binds x:numVar, None arm covers the rest, both bodies numeric → unify → default Int64).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "match".toUTF8,
                            .name "Some".toUTF8, .intLit false .dec (ByteArray.mk #[5]), .name "x".toUTF8,
                            .name "None".toUTF8, .name "_".toUTF8, .intLit false .dec (ByteArray.mk #[0]),
                            .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .list #[0, 1], .atom 4, .atom 6, .list #[3, 4], .atom 6,
                           .list #[5, 6], .atom 7, .atom 8, .list #[8, 9], .atom 9, .list #[10, 11],
                           .atom 3, .list #[13, 2, 7, 12], .atom 2, .list #[15], .atom 1, .list #[17, 16, 14],
                           .atom 10, .atom 2, .list #[19, 20], .atom 0, .list #[22, 18, 21]],
                root := 23 } == .wellTyped (.int 64 true))
-- T1.17 (Mat exhaustiveness): NON-exhaustive `(match (Some 5) ((Some x) x))` (None uncovered, no
-- catch-all) → IllTyped CDZ0210 (rcdzc rejects a non-exhaustive sum match; a genuinely non-exhaustive
-- match is never accepted, so this only ever agrees — never a false accept).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "match".toUTF8,
                            .name "Some".toUTF8, .intLit false .dec (ByteArray.mk #[5]), .name "x".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .list #[0, 1], .atom 4, .atom 6, .list #[3, 4], .atom 6,
                           .list #[5, 6], .atom 3, .list #[8, 2, 7], .atom 2, .list #[10], .atom 1,
                           .list #[12, 11, 9], .atom 7, .atom 2, .list #[14, 15], .atom 0, .list #[17, 13, 16]],
                root := 18 } == .illTyped "CDZ0210")
-- T1.18 (let-generalization): `(let ((id (fn (x) x))) (id 5))` → WellTyped Int64. The let-bound `id`
-- generalizes to `∀α.α→α`; the V rule instantiates a fresh var at the use, App unifies it with the
-- numeric arg → Int64 (defaulted). Exercises generalize → instantiate → App on a let-bound fn.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "let".toUTF8,
                            .name "id".toUTF8, .name "fn".toUTF8, .name "x".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 6, .list #[0], .atom 6, .atom 5, .list #[3, 1, 2], .atom 4, .list #[5, 4],
                           .list #[6], .atom 4, .atom 7, .list #[8, 9], .atom 3, .list #[11, 7, 10],
                           .atom 2, .list #[13], .atom 1, .list #[15, 14, 12], .atom 8, .atom 2,
                           .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                root := 21 } == .wellTyped (.int 64 true))
-- T1.19 (sum-type annotation): `(: (Some 5) (Option Int64))` → WellTyped (Option Int64). parseTy? now
-- parses `(Option T)`; the ascription unifies the Some-payload numVar with Int64 → a determined Option.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .name "Some".toUTF8, .intLit false .dec (ByteArray.mk #[5]), .name "Option".toUTF8,
                            .name "Int64".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .list #[0, 1], .atom 6, .atom 7, .list #[3, 4], .atom 3,
                           .list #[6, 2, 5], .atom 2, .list #[8], .atom 1, .list #[10, 9, 7], .atom 8,
                           .atom 2, .list #[12, 13], .atom 0, .list #[15, 11, 14]],
                root := 16 } == .wellTyped (optionTy (.int 64 true)))
-- T1.21 (local fn-def): `(def (main) (do (def (inc x) (+ x 1)) (inc 5)))` → WellTyped Int64. The do-local
-- fn-def `(def (inc x) …)` types via the Fn logic (inc : numVar→numVar), binds it, then `(inc 5)` applies.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "inc".toUTF8,
                            .name "x".toUTF8, .name "+".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .list #[0, 1], .atom 5, .atom 4, .atom 6, .list #[3, 4, 5],
                           .atom 1, .list #[7, 2, 6], .atom 3, .atom 7, .list #[9, 10], .atom 0,
                           .list #[12, 8, 11], .atom 2, .list #[14], .atom 1, .list #[16, 15, 13],
                           .atom 8, .atom 2, .list #[18, 19], .atom 0, .list #[21, 17, 20]],
                root := 22 } == .wellTyped (.int 64 true))
-- T1.23 (function-type annotation): `(: (fn (x) x) (-> Int64 Int64))` → WellTyped Int64→Int64. parseTy?
-- now parses `(-> A B)`; the ascription unifies the fn's param/result vars with the annotated arrow.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .name "fn".toUTF8, .name "x".toUTF8, .name "->".toUTF8, .name "Int64".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 5, .list #[0], .atom 5, .atom 4, .list #[3, 1, 2], .atom 6, .atom 7,
                           .atom 7, .list #[5, 6, 7], .atom 3, .list #[9, 4, 8], .atom 2, .list #[11],
                           .atom 1, .list #[13, 12, 10], .atom 8, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.fn (.int 64 true) (.int 64 true)))
-- T1.24 (top-level fn-def): `(do (def (inc x) (+ x 1)) (def (main) (inc 5)) (export main))` → WellTyped
-- Int64. The sibling `inc` is typed into the top-level env (numVar→numVar), then `main`'s body `(inc 5)`
-- resolves it. Also validates the InferState is THREADED from topLevelEnv into main (else inc's numVar id
-- would collide with main's fresh vars).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "inc".toUTF8, .name "x".toUTF8,
                            .name "+".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .name "main".toUTF8,
                            .intLit false .dec (ByteArray.mk #[5]), .name "export".toUTF8],
                nodes := #[.atom 2, .atom 3, .list #[0, 1], .atom 4, .atom 3, .atom 5, .list #[3, 4, 5],
                           .atom 1, .list #[7, 2, 6], .atom 6, .list #[9], .atom 2, .atom 7, .list #[11, 12],
                           .atom 1, .list #[14, 10, 13], .atom 8, .atom 6, .list #[16, 17], .atom 0,
                           .list #[19, 8, 15, 18]],
                root := 20 } == .wellTyped (.int 64 true))
-- T1.25 (user sum): `(do (type P (Mk Int64) (Z)) (def (main) (match (Mk 5) ((Mk n) n) ((Z) 0))) (export main))`
-- → WellTyped Int64. `(Mk 5)` constructs the P sum (Mk payload Int64); the match binds n:Int64, both arms
-- numeric, exhaustive over {Mk,Z} → Int64. Exercises userSumMap (structural sum + no-collision gate) + Con.
#guard (infer { leaves := #[.name "do".toUTF8, .name "type".toUTF8, .name "P".toUTF8, .name "Mk".toUTF8,
                            .name "Int64".toUTF8, .name "Z".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                            .name "match".toUTF8, .intLit false .dec (ByteArray.mk #[5]), .name "n".toUTF8,
                            .intLit false .dec (ByteArray.mk #[0]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .list #[0, 1], .atom 5, .list #[3], .atom 1, .atom 2,
                           .list #[5, 6, 2, 4], .atom 3, .atom 9, .list #[8, 9], .atom 3, .atom 10,
                           .list #[11, 12], .atom 10, .list #[13, 14], .atom 5, .list #[16], .atom 11,
                           .list #[17, 18], .atom 8, .list #[20, 10, 15, 19], .atom 7, .list #[22],
                           .atom 6, .list #[24, 23, 21], .atom 12, .atom 7, .list #[26, 27], .atom 0,
                           .list #[29, 7, 25, 28]],
                root := 30 } == .wellTyped (.int 64 true))
-- T1.26 (user sum, bare-name nullary ctor): `(do (type C R G B) (def (main) (match R (R 1) (G 2) (B 3)))
-- (export main))` → WellTyped Int64. A bare enum variant `R` in value position constructs its type's sum;
-- the match's bare `R`/`G`/`B` patterns cover {R,G,B} exhaustively; numeric arms → Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "type".toUTF8, .name "C".toUTF8, .name "R".toUTF8,
                            .name "G".toUTF8, .name "B".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                            .name "match".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .intLit false .dec (ByteArray.mk #[3]),
                            .name "export".toUTF8],
                nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .atom 5, .list #[0, 1, 2, 3, 4], .atom 3,
                           .atom 3, .atom 9, .list #[7, 8], .atom 4, .atom 10, .list #[10, 11], .atom 5,
                           .atom 11, .list #[13, 14], .atom 8, .list #[16, 6, 9, 12, 15], .atom 7,
                           .list #[18], .atom 6, .list #[20, 19, 17], .atom 12, .atom 7, .list #[22, 23],
                           .atom 0, .list #[25, 5, 21, 24]],
                root := 26 } == .wellTyped (.int 64 true))
-- T1.27 (user sum, QUALIFIED ctor): `(do (type C R G B) (def (main) (match C.R (R 1) (G 2) (B 3))) (export main))`
-- → WellTyped Int64. The scrutinee `C.R` = `(. C R)` is a qualified nullary variant (its declaring type is
-- `C`) → the C sum; bare `R`/`G`/`B` patterns cover it exhaustively → Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "type".toUTF8, .name "C".toUTF8, .name "R".toUTF8,
                            .name "G".toUTF8, .name "B".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                            .name "match".toUTF8, .name ".".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .intLit false .dec (ByteArray.mk #[3]),
                            .name "export".toUTF8],
                nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .atom 5, .list #[0, 1, 2, 3, 4], .atom 9,
                           .atom 2, .atom 3, .list #[6, 7, 8], .atom 3, .atom 10, .list #[10, 11], .atom 4,
                           .atom 11, .list #[13, 14], .atom 5, .atom 12, .list #[16, 17], .atom 8,
                           .list #[19, 9, 12, 15, 18], .atom 7, .list #[21], .atom 6, .list #[23, 22, 20],
                           .atom 13, .atom 7, .list #[25, 26], .atom 0, .list #[28, 5, 24, 27]],
                root := 29 } == .wellTyped (.int 64 true))
-- T1.28 (user sum, QUALIFIED patterns): `(do (type C R G B) (def (main) (match C.R (C.R 1) (C.G 2) (C.B 3)))
-- (export main))` → WellTyped Int64. The common corpus form: both the scrutinee AND the arm patterns are
-- qualified `C.R`/`C.G`/`C.B` = `(. C _)`; each covers its variant of C, exhaustive → Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "type".toUTF8, .name "C".toUTF8, .name "R".toUTF8,
                            .name "G".toUTF8, .name "B".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                            .name "match".toUTF8, .name ".".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .intLit false .dec (ByteArray.mk #[3]),
                            .name "export".toUTF8],
                nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .atom 5, .list #[0, 1, 2, 3, 4],
                           .atom 9, .atom 2, .atom 3, .list #[6, 7, 8], .atom 9, .atom 2, .atom 3,
                           .list #[10, 11, 12], .atom 10, .list #[13, 14], .atom 9, .atom 2, .atom 4,
                           .list #[16, 17, 18], .atom 11, .list #[19, 20], .atom 9, .atom 2, .atom 5,
                           .list #[22, 23, 24], .atom 12, .list #[25, 26], .atom 8,
                           .list #[28, 9, 15, 21, 27], .atom 7, .list #[30], .atom 6, .list #[32, 31, 29],
                           .atom 13, .atom 7, .list #[34, 35], .atom 0, .list #[37, 5, 33, 36]],
                root := 38 } == .wellTyped (.int 64 true))
-- T1.29 (List construction): `(do (def (main) (list 1 2 3)) (export main))` → WellTyped (List Int64).
-- The three int elements unify to one numeric element type, defaulting to Int64 → `.listTy Int64`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "list".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .list #[0, 1, 2, 3], .atom 2, .list #[5],
                           .atom 1, .list #[7, 6, 4], .atom 7, .atom 2, .list #[9, 10], .atom 0,
                           .list #[12, 8, 11]],
                root := 13 } == .wellTyped (.listTy (.int 64 true)))
-- T1.30 (List op): `(do (def (main) (List.len (list 1 2 3))) (export main))` → WellTyped Int64. `List.len`
-- = `((. List len) …)`; the arg unifies with `List β` → the op yields Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "List".toUTF8, .name "len".toUTF8, .name "list".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .atom 8, .atom 9,
                           .list #[4, 5, 6, 7], .list #[3, 8], .atom 2, .list #[10], .atom 1,
                           .list #[12, 11, 9], .atom 10, .atom 2, .list #[14, 15], .atom 0,
                           .list #[17, 13, 16]],
                root := 18 } == .wellTyped (.int 64 true))
-- T1.31 (List op): `(do (def (main) (List.concat (list 1) (list 2))) (export main))` → WellTyped (List Int64).
-- Both list args unify to `List β`; the numeric elements default to Int64 → `.listTy Int64`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "List".toUTF8, .name "concat".toUTF8, .name "list".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .list #[4, 5],
                           .atom 6, .atom 8, .list #[7, 8], .list #[3, 6, 9], .atom 2, .list #[11],
                           .atom 1, .list #[13, 12, 10], .atom 9, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.listTy (.int 64 true)))
-- T1.34 (List.at is TOTAL-FALLIBLE): `(do (def (main) (List.at (list 1 2 3) 0)) (export main))` →
-- WellTyped (Option Int64) — NOT bare Int64 (the T1.30 false-accept v-cdz-smith --typegen caught). The
-- element defaults to Int64, indexing yields `(Option Int64)`; the Some payload is determined so it does
-- not trip the undetermined-sum escape guard.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "List".toUTF8, .name "at".toUTF8, .name "list".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .intLit false .dec (ByteArray.mk #[0]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .atom 8, .atom 9,
                           .list #[4, 5, 6, 7], .atom 10, .list #[3, 8, 9], .atom 2, .list #[11], .atom 1,
                           .list #[13, 12, 10], .atom 11, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (optionTy (.int 64 true)))
-- T1.32 (Set construction): `(do (def (main) (set 1 2 3)) (export main))` → WellTyped (Set Int64). Like a
-- list — the three int elements unify to one element type, defaulting to Int64 → `.setTy Int64`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "set".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .atom 6, .list #[0, 1, 2, 3], .atom 2, .list #[5],
                           .atom 1, .list #[7, 6, 4], .atom 7, .atom 2, .list #[9, 10], .atom 0,
                           .list #[12, 8, 11]],
                root := 13 } == .wellTyped (.setTy (.int 64 true)))
-- T1.32 (Set op): `(do (def (main) (Set.len (set 1 2 3))) (export main))` → WellTyped Int64. `Set.len` =
-- `((. Set len) …)`; the arg unifies with `Set β` → the op yields Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Set".toUTF8, .name "len".toUTF8, .name "set".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .intLit false .dec (ByteArray.mk #[3]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .atom 8, .atom 9,
                           .list #[4, 5, 6, 7], .list #[3, 8], .atom 2, .list #[10], .atom 1,
                           .list #[12, 11, 9], .atom 10, .atom 2, .list #[14, 15], .atom 0,
                           .list #[17, 13, 16]],
                root := 18 } == .wellTyped (.int 64 true))
-- T1.33 (Map construction): `(do (def (main) (map (= 1 2))) (export main))` → WellTyped (Map Int64 Int64).
-- The single entry's key and value are ints → K,V default to Int64 → `.mapTy Int64 Int64`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "map".toUTF8,
                            .name "=".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2], .atom 3, .list #[4, 3], .atom 2,
                           .list #[6], .atom 1, .list #[8, 7, 5], .atom 7, .atom 2, .list #[10, 11], .atom 0,
                           .list #[13, 9, 12]],
                root := 14 } == .wellTyped (.mapTy (.int 64 true) (.int 64 true)))
-- T1.33 (Map op): `(do (def (main) (Map.len (map (= 1 2)))) (export main))` → WellTyped Int64. `Map.len` =
-- `((. Map len) …)`; the arg unifies with `(Map K V)` → the op yields Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Map".toUTF8, .name "len".toUTF8, .name "map".toUTF8, .name "=".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 7, .atom 8, .atom 9,
                           .list #[4, 5, 6], .atom 6, .list #[8, 7], .list #[3, 9], .atom 2, .list #[11],
                           .atom 1, .list #[13, 12, 10], .atom 10, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.int 64 true))
-- T1.35 (Map.empty in context): `(do (def (main) (Map.insert Map.empty 1 2)) (export main))` → WellTyped
-- (Map Int64 Int64). `Map.empty` = `(. Map empty)` → `.mapTy (var)(var)`; `insert` unifies its k/v with the
-- int args → determined `.mapTy Int64 Int64` (a BARE `Map.empty` escape would instead decline, undetermined).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Map".toUTF8, .name "insert".toUTF8, .name "empty".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 3, .atom 4, .atom 6,
                           .list #[4, 5, 6], .atom 7, .atom 8, .list #[3, 7, 8, 9], .atom 2, .list #[11],
                           .atom 1, .list #[13, 12, 10], .atom 9, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.mapTy (.int 64 true) (.int 64 true)))
-- T1.36 (Tuple type annotation): `(do (def (main) (: (tuple 1 2) (Tuple Int64 Int64))) (export main))` →
-- WellTyped (Tuple Int64 Int64). `(Tuple T…)` in parseTy? → `.tuple [T…]`; the ascription unifies the
-- `(tuple 1 2)` value (`.tuple [numVar, numVar]`) with it → `.tuple [Int64, Int64]`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .name "tuple".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "Tuple".toUTF8,
                            .name "Int64".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .list #[0, 1, 2], .atom 7, .atom 8, .atom 8,
                           .list #[4, 5, 6], .atom 3, .list #[8, 3, 7], .atom 2, .list #[10], .atom 1,
                           .list #[12, 11, 9], .atom 9, .atom 2, .list #[14, 15], .atom 0,
                           .list #[17, 13, 16]],
                root := 18 } == .wellTyped (.tuple [.int 64 true, .int 64 true]))
-- T1.37 (Record type annotation): `(do (def (main) (: (record (= x 1)) (Record (: x Int64)))) (export main))`
-- → WellTyped (Record x:Int64). `(Record (: k T)…)` in parseTy? → `.record [(k,T)…]` (sorted); the
-- ascription unifies the `(record (= x 1))` value (`.record [(x, numVar)]`) with it → `.record [(x, Int64)]`.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                            .name "record".toUTF8, .name "=".toUTF8, .name "x".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .name "Record".toUTF8,
                            .name "Int64".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 4, .atom 5, .atom 6, .atom 7, .list #[1, 2, 3], .list #[0, 4], .atom 8,
                           .atom 3, .atom 6, .atom 9, .list #[7, 8, 9], .list #[6, 10], .atom 3,
                           .list #[12, 5, 11], .atom 2, .list #[14], .atom 1, .list #[16, 15, 13],
                           .atom 10, .atom 2, .list #[18, 19], .atom 0, .list #[21, 17, 20]],
                root := 22 } == .wellTyped (.record [("x".toUTF8, .int 64 true)]))
-- T1.38 (String op): `(do (def (main) (String.byte-len "hi")) (export main))` → WellTyped Int64.
-- `String.byte-len` = `((. String byte-len) …)`; the receiver unifies with String → the op yields Int64.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "String".toUTF8, .name "byte-len".toUTF8, .str "hi".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .list #[3, 4], .atom 2,
                           .list #[6], .atom 1, .list #[8, 7, 5], .atom 7, .atom 2, .list #[10, 11], .atom 0,
                           .list #[13, 9, 12]],
                root := 14 } == .wellTyped (.int 64 true))
-- T1.39 (Bytes op + String↔Bytes bridge): `(do (def (main) (Bytes.len (String.to-bytes "hi"))) (export main))`
-- → WellTyped Int64. `String.to-bytes "hi"` → Bytes; `Bytes.len` on it → Int64 (exercises the new `.bytes` Ty).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Bytes".toUTF8, .name "len".toUTF8, .name "String".toUTF8,
                            .name "to-bytes".toUTF8, .str "hi".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 3, .atom 6, .atom 7,
                           .list #[4, 5, 6], .atom 8, .list #[7, 8], .list #[3, 9], .atom 2, .list #[11],
                           .atom 1, .list #[13, 12, 10], .atom 9, .atom 2, .list #[15, 16], .atom 0,
                           .list #[18, 14, 17]],
                root := 19 } == .wellTyped (.int 64 true))
-- T1.40 (Float literal): `(do (def (main) nan) (export main))` → WellTyped Float64. A float literal (here
-- `nan`) is width-polymorphic (`.floatVar`), defaulting to Float64 at escape (01-literals:329).
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .floatNan,
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 2, .list #[1], .atom 1, .list #[3, 2, 0], .atom 4, .atom 2,
                           .list #[5, 6], .atom 0, .list #[8, 4, 7]],
                root := 9 } == .wellTyped (.float 64))
-- T1.41 (Float arithmetic): `(do (def (main) (+ nan nan)) (export main))` → WellTyped Float64. `+` on two
-- float literals stays width-poly (floatVar), defaulting to Float64 at escape. (`%` on a float → CDZ0301.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name "+".toUTF8,
                            .floatNan, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 4, .list #[0, 1, 2], .atom 2, .list #[4], .atom 1,
                           .list #[6, 5, 3], .atom 5, .atom 2, .list #[8, 9], .atom 0, .list #[11, 7, 10]],
                root := 12 } == .wellTyped (.float 64))
-- T1.42 (Float op): `(do (def (main) (Float64.of-int 5)) (export main))` → WellTyped Float64. `of-int`
-- converts an int (here the literal 5) to this width's float → Float64. (`Float64.nan` const → Float64 too.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Float64".toUTF8, .name "of-int".toUTF8, .intLit false .dec (ByteArray.mk #[5]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .list #[3, 4], .atom 2,
                           .list #[6], .atom 1, .list #[8, 7, 5], .atom 7, .atom 2, .list #[10, 11], .atom 0,
                           .list #[13, 9, 12]],
                root := 14 } == .wellTyped (.float 64))
-- T1.43 (int-module op): `(do (def (main) (Int64.wrapping-add 1 2)) (export main))` → WellTyped Int64.
-- Both operands unify with this module's width (Int64) → the wrapping op yields Int64. (min/max constants +
-- of/wrap/checked-* are the other int-module members.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Int64".toUTF8, .name "wrapping-add".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .list #[3, 4, 5],
                           .atom 2, .list #[7], .atom 1, .list #[9, 8, 6], .atom 8, .atom 2, .list #[11, 12],
                           .atom 0, .list #[14, 10, 13]],
                root := 15 } == .wellTyped (.int 64 true))
-- T1.44 (Rational op): `(do (def (main) (Rational.of 1 2)) (export main))` → WellTyped Rational.
-- `of` takes two ints (numerator, denominator) → an exact Rational.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Rational".toUTF8, .name "of".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .list #[3, 4, 5],
                           .atom 2, .list #[7], .atom 1, .list #[9, 8, 6], .atom 8, .atom 2, .list #[11, 12],
                           .atom 0, .list #[14, 10, 13]],
                root := 15 } == .wellTyped .rational)
-- T1.45 (Char op): `(do (def (main) (Char.to-int #"a")) (export main))` → WellTyped Int64.
-- `to-int` reads a Char's scalar value → Int64. (`from-int (Int64) → (Option Char)` is the fallible inverse.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Char".toUTF8, .name "to-int".toUTF8, .char "a".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .list #[3, 4], .atom 2,
                           .list #[6], .atom 1, .list #[8, 7, 5], .atom 7, .atom 2, .list #[10, 11], .atom 0,
                           .list #[13, 9, 12]],
                root := 14 } == .wellTyped (.int 64 true))
-- T1.46 (BigInt op): `(do (def (main) (BigInt.of 5)) (export main))` → WellTyped BigInt.
-- `of` widens a fixed-width int to arbitrary precision. (BigInt arith incl. `%` → BigInt; a numeric literal
-- annotated BigInt grounds; Rational.numerator/denominator now → BigInt.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "BigInt".toUTF8, .name "of".toUTF8, .intLit false .dec (ByteArray.mk #[5]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .list #[3, 4], .atom 2,
                           .list #[6], .atom 1, .list #[8, 7, 5], .atom 7, .atom 2, .list #[10, 11], .atom 0,
                           .list #[13, 9, 12]],
                root := 14 } == .wellTyped .bigint)
-- T1.47 (Tuple op): `(do (def (main) (Tuple.size (tuple 1 2))) (export main))` → WellTyped Int64.
-- `size` reads the tuple's arity. (`concat`/`cat` append two tuples' element lists → a wider Tuple.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Tuple".toUTF8, .name "size".toUTF8, .name "tuple".toUTF8,
                            .intLit false .dec (ByteArray.mk #[1]), .intLit false .dec (ByteArray.mk #[2]),
                            .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .atom 7, .atom 8,
                           .list #[4, 5, 6], .list #[3, 7], .atom 2, .list #[9], .atom 1, .list #[11, 10, 8],
                           .atom 9, .atom 2, .list #[13, 14], .atom 0, .list #[16, 12, 15]],
                root := 17 } == .wellTyped (.int 64 true))
-- T1.48 (Record row op): `(do (def (main) (Record.with (record (= x 1)) #"x" #t)) (export main))` →
-- WellTyped (Record x:Bool). `with` replaces the PRESENT field `x` — its type becomes the value's (Bool),
-- a new record type. (`extend` would ADD an absent field; an absent `with`/present `extend` → CDZ0212/0211.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "with".toUTF8, .name "record".toUTF8, .name "=".toUTF8,
                            .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .sym "x".toUTF8,
                            .boolLit true, .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 10, .atom 11, .list #[9, 5, 10, 11],
                           .atom 2, .list #[13], .atom 1, .list #[15, 14, 12], .atom 12, .atom 2,
                           .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                root := 21 } == .wellTyped (.record [("x".toUTF8, .bool)]))
-- T1.49 (Record.pop): `(do (def (main) (Record.pop (record (= x 1)) #"x")) (export main))` → WellTyped
-- (Tuple Int64 (Record)) — the popped field's type paired with the record minus x (here empty). (`merge`
-- unions two disjoint records; an overlapping merge / an absent-key pop is CDZ0211 / CDZ0212.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "pop".toUTF8, .name "record".toUTF8, .name "=".toUTF8,
                            .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .sym "x".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 10, .list #[9, 5, 10], .atom 2,
                           .list #[12], .atom 1, .list #[14, 13, 11], .atom 11, .atom 2, .list #[16, 17],
                           .atom 0, .list #[19, 15, 18]],
                root := 20 } == .wellTyped (.tuple [.int 64 true, .record []]))
-- T1.50 (Record.project): `(do (def (main) (Record.project (record (= x 1) (= y 2)) (x))) (export main))` →
-- WellTyped (Record x:Int64) — narrow to the named labels (drops y). The 2nd operand `(x)` is a LITERAL
-- label list (bare names). (`without` is the complement; an absent label → CDZ0212, a duplicate → CDZ0201.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "project".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "y".toUTF8, .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .atom 7, .atom 10, .atom 11,
                           .list #[5, 6, 7], .list #[0, 4, 8], .atom 3, .atom 4, .atom 5, .list #[10, 11, 12],
                           .atom 8, .list #[14], .list #[13, 9, 15], .atom 2, .list #[17], .atom 1,
                           .list #[19, 18, 16], .atom 12, .atom 2, .list #[21, 22], .atom 0,
                           .list #[24, 20, 23]],
                root := 25 } == .wellTyped (.record [("x".toUTF8, .int 64 true)]))
-- T1.50-fault (Record.project ABSENT label): `(Record.project (record (= x 1)) (y))` → IllTyped CDZ0212 —
-- `y` names no field of `{x}` (the absent-field rejection, `infer/node.rs`). Pins the project fault path.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "project".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "y".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 10, .list #[10], .list #[9, 5, 11],
                           .atom 2, .list #[13], .atom 1, .list #[15, 14, 12], .atom 11, .atom 2,
                           .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                root := 21 } == .illTyped "CDZ0212")
-- T1.49-fault (Record.merge OVERLAPPING): `(Record.merge (record (= x 1)) (record (= x 2)))` → IllTyped
-- CDZ0211 — both records share `x` (merge is disjoint-only, not last-writer-wins). Pins the merge fault path.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "merge".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .intLit false .dec (ByteArray.mk #[2]), .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 6,
                           .atom 7, .atom 8, .atom 10, .list #[7, 8, 9], .list #[6, 10], .atom 3, .atom 4,
                           .atom 5, .list #[12, 13, 14], .list #[15, 5, 11], .atom 2, .list #[17], .atom 1,
                           .list #[19, 18, 16], .atom 11, .atom 2, .list #[21, 22], .atom 0,
                           .list #[24, 20, 23]],
                root := 25 } == .illTyped "CDZ0211")
-- T1.49-fault (Record.pop ABSENT key): `(Record.pop (record (= x 1)) #"y")` → IllTyped CDZ0212 — `#"y"`
-- names no field of `{x}` (the field access fails). Pins the pop fault path.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "pop".toUTF8, .name "record".toUTF8, .name "=".toUTF8,
                            .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]), .sym "y".toUTF8,
                            .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 10, .list #[9, 5, 10], .atom 2,
                           .list #[12], .atom 1, .list #[14, 13, 11], .atom 11, .atom 2, .list #[16, 17],
                           .atom 0, .list #[19, 15, 18]],
                root := 20 } == .illTyped "CDZ0212")
-- T1.50-fault (Record.without ABSENT label): `(Record.without (record (= x 1)) (y))` → IllTyped CDZ0212 —
-- `y` names no field of `{x}` (same absent-field check as project). Pins the without fault path.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "without".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "y".toUTF8, .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 10, .list #[10], .list #[9, 5, 11],
                           .atom 2, .list #[13], .atom 1, .list #[15, 14, 12], .atom 11, .atom 2,
                           .list #[17, 18], .atom 0, .list #[20, 16, 19]],
                root := 21 } == .illTyped "CDZ0212")
-- T1.50-fault (Record.project DUPLICATE label): `(Record.project (record (= x 1)) (x x))` → IllTyped
-- CDZ0201 — a label named twice is ill-formed (a record's fields are a fixed SET). Pins the dup-label path.
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Record".toUTF8, .name "project".toUTF8, .name "record".toUTF8,
                            .name "=".toUTF8, .name "x".toUTF8, .intLit false .dec (ByteArray.mk #[1]),
                            .name "export".toUTF8],
                nodes := #[.atom 6, .atom 7, .atom 8, .atom 9, .list #[1, 2, 3], .list #[0, 4], .atom 3,
                           .atom 4, .atom 5, .list #[6, 7, 8], .atom 8, .atom 8, .list #[10, 11],
                           .list #[9, 5, 12], .atom 2, .list #[14], .atom 1, .list #[16, 15, 13], .atom 10,
                           .atom 2, .list #[18, 19], .atom 0, .list #[21, 17, 20]],
                root := 22 } == .illTyped "CDZ0201")
-- T1.51 (built-in op PARTIAL APPLICATION, #8313): `(do (def (main) ((Int64.wrapping-add 3) 4)) (export main))`
-- → WellTyped Int64. `(Int64.wrapping-add 3)` curries to a closure `Int64→Int64` (partial), then applying it
-- to `4` (a non-name expression head) yields Int64. Exercises both the partial-→.fn arm AND the apply-a-
-- fn-typed-expression-head rule. (rcdzc accepts partial application; this widens the oracle to match + feeds
-- a --typegen arm for the closure-currying path.)
#guard (infer { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ".".toUTF8,
                            .name "Int64".toUTF8, .name "wrapping-add".toUTF8, .intLit false .dec (ByteArray.mk #[3]),
                            .intLit false .dec (ByteArray.mk #[4]), .name "export".toUTF8],
                nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 6, .list #[3, 4], .atom 7,
                           .list #[5, 6], .atom 2, .list #[8], .atom 1, .list #[10, 9, 7], .atom 8, .atom 2,
                           .list #[12, 13], .atom 0, .list #[15, 11, 14]],
                root := 16 } == .wellTyped (.int 64 true))
-- accept ∧ well-typed → agree
#guard judgeTypecheck (.wellTyped .bool) .accept == .holds
-- both reject (any code) → agree (T1); decline ∧ ill-typed → agree
#guard judgeTypecheck (.illTyped "CDZ0203") (.reject "CDZ0201") == .holds
#guard judgeTypecheck (.illTyped "CDZ0203") .decline == .holds
-- a WellTyped vs a NON-type reject (overflow/malformed/pragma) is NOT a false-reject → skip (the program
-- IS well-typed; rcdzc rejected it in a phase outside the type oracle's remit).
#guard (match judgeTypecheck (.wellTyped (.int 64 true)) (.reject "CDZ0304") with | .skip _ => true | _ => false)
#guard (match judgeTypecheck (.wellTyped .unit) (.reject "CDZ0201") with | .skip _ => true | _ => false)
-- FALSE-REJECT — the highest-value finding: oracle accepts, rcdzc coded-rejected
#guard judgeTypecheck (.wellTyped .unit) (.reject "CDZ0203")
       == .mismatch "false-reject: oracle infers well-typed, rcdzc rejected CDZ0203"
-- CAPABILITY-GAP — oracle accepts, rcdzc codeless-declined (should-work-not-yet-built)
#guard judgeTypecheck (.wellTyped .unit) .decline
       == .mismatch "capability-gap: oracle infers well-typed, rcdzc declined (should-work-not-yet-built)"
-- FALSE-ACCEPT / soundness hole — oracle rejects, rcdzc accepted (reached once inference lands)
#guard (judgeTypecheck (.illTyped "CDZ0203") .accept != .holds)

end Oracle
