# DESIGN: recursive-generic monomorphization (instantiate a recursive def more than once)

Status: COMPLETE — a recursive generic function is instantiated once per distinct concrete type, end to
end, across EVERY recursive-def flavor: top-level, transitive (generic-calls-generic chains), mutual
recursion, do-local, and module-member. Same-type calls dedup; unbounded polymorphic recursion rejects
cleanly at type-check. Works over USER-DEFINED GENERIC RECURSIVE SUM TYPES too — `(type Lst Nil (Cons a
(Lst a)))` with an unannotated `len` monomorphizes per element type (`Lst Int64` + `Lst String` → 5),
the recursive-DATA idiom, not just scalar pass-through. All follow-ons resolved. 0 fail throughout.

ONE adjacent gap, a SEPARATE feature (NOT monomorphization): an explicit POLYMORPHIC ANNOTATION
`(: l (Lst a))` / `(: b (Box a))` is rejected — the type variable `a` is unbound in the annotation's
scope (there is no form binding a signature's type variables). A CONCRETE generic-type annotation
`(: b (Box Int64))` works, and the UNANNOTATED polymorphic form works (inference carries the element
type — the idiomatic spelling). Binding a signature's type variables so a polymorphic annotation resolves
is its own increment (type-variable-in-signature), orthogonal to the monomorphization engine.

## Transitive genericity (LANDED after the initial phases) — generic-calls-generic

A generic recursive fn (`wrap`) that CALLS another generic (`idr`) threading its own param — `wrap`'s
result IS `idr`'s result IS the threaded param's type — must itself be generic. Two fixes, both in
`infer.rs`:
- **Detection propagates through the call graph** (`call_site_distinct_arg_types`): when an argument
  types as a `Var`/`Any` but is ANOTHER def's parameter (`arg_is_other_def_param`), that caller-param's
  own distinct-type spread flows through (`seed_transitive`-guarded against a cycle). So `idr`, called at
  ONE syntactic site with `wrap`'s generic `y`, inherits `{Int64,String}` → detected generic.
- **The param↔result var connection survives** (`apply_scheme_to_args`): for a GENERIC callee scheme,
  seed the instantiation `Fresh` counter PAST every var the args carry and SKIP the arg `freshen_free` —
  so a threaded bare param-var arg flows through the unify untouched and the callee's result var (equal
  to its param var) resolves to the caller's param var. Without this a chain decoupled `(-> Int64 (-> ?a
  ?a))` into `(-> Int64 (-> ?a ?b))` → "looped function result has no machine rep". A MONOMORPHIC scheme
  keeps the exact old freshen path (byte-identical).

## What landed (the mechanism, as built)

- **Phase 1 — generalize the scheme** (`infer.rs`). `solve_recursive_params` detects a GENUINELY-GENERIC
  parameter — one the body only threads (still a free `Var` after the body walk) AND that callers invoke
  at ≥2 distinct concrete types (`generic_param_positions` + `call_site_distinct_arg_types`) — and leaves
  it a canonical `Ty::Var` instead of pinning it to the first call site. `compute_def_scheme` then
  quantifies the signature's free vars (`Ty::collect_free_vars`) into `Scheme.ty_vars`, so a recursive
  call types by a POLYMORPHIC scheme and each site `instantiate`s fresh — the CDZ0203 is gone. A param
  called at ≤1 type stays monomorphic (byte-identical to before). A no-free-var def → `Scheme::mono`.
- **Phase 2 — specialize by copy** (`lower.rs::type_specialize`, modeled on `effects::specialize_recursive`
  minus the state params). At a recursive `Core::Call` whose callee scheme is generic, compute the
  concrete arg types at THIS site, synthesize (memoized on `(body, rendered-sig)`) a copy of the def whose
  params are re-annotated with those concrete types (`eval::copy_structural_pub`), and point the
  `Core::Call` at the copy. `layout` emits it as an ordinary monomorphic function; `def_params`/`valtype_of`
  give it real machine valtypes. The self-call in the copied body re-resolves by name to the original and
  re-enters `type_specialize` with the same instantiation → hits the memo (recursion closes at the
  specialization). **Dedup** is automatic: the memo key IS the concrete signature, so two calls at the same
  type share one function.
- `db.type_specializations` memo; `Ty::collect_free_vars`; `eval::copy_structural_pub`.

## Phase 0 findings (verified in the `recmono` worktree, 2026-07-14)

Reproduction (rep-SENSITIVE — proves copy-specialization is required, not optional):
```
(def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))   ; x threads through, generic
(def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 "hi"))))
```
→ `CDZ0203: String and Int64 must be the same type here`. `loopn` at Int64 (i64 slot) AND String
(i32 handle) — different valtypes, so a single shared function is impossible; each needs its own.

The mechanism, traced end-to-end (`CDZ_LOG=rcdzc::infer=trace`):
1. `solve_recursive_params` (`infer.rs:1029`) **grounds** `loopn`'s generic param `x` to `Int64` —
   pinned by the FIRST call site via CALL-SITE SEEDING (`infer.rs:1066`, landed `cea7ab44`). This is
   the pin. `db.param_types[x] = Int64`.
2. `compute_def_scheme` (`infer.rs:1934`) reads that and returns `Scheme::mono((-> Int64 (-> Int64
   Int64)))` — "determined monomorphic signature". The generic `x` is gone.
3. At the String call, the call-site arg check (`infer.rs:4188`, `type_of(param_occ)` vs
   `type_of(arg)`) unifies `Int64` (pinned param) with `String` (arg) → CDZ0203.

Key seams confirmed:
- **`instantiate` (`unify.rs:597`) ALREADY freshens a quantified scheme** — so populating
  `Scheme.ty_vars`/`width_vars`/`sign_vars` (fields exist, `ty.rs:1377`) is sufficient to make a
  recursive-generic call type-check. `apply_scheme_to_args` (`infer.rs:2657`) already
  instantiate-then-unifies, so each call site gets a private instantiation for free.
- **`type_errors` runs on EVERY def body, reachable or not** (`compile.rs:1111`). A generic template
  body types fine with a free-var param (`loopn`'s `x:a` self-call instantiates), so generalizing
  creates no spurious template errors.
- **The pin and the type error are INSEPARABLE.** Call-site seeding forces one mono type; NOT seeding
  (generalizing) makes the type-check pass but leaves the param a `Var` with no machine valtype
  (`valtype_of(Ty::Var) = None`, `lir.rs:384`). So generalization REQUIRES per-instantiation copy to
  recover a concrete valtype at emission — Phase 1 and Phase 2 land TOGETHER for a rep-sensitive def.
  A rep-UNIFORM def (all instantiations share one valtype) could ship a single function, but the corpus
  has no clean rep-uniform generic recursion (built-in `List` match unsupported; user sums are
  monomorphic). So we build the GENERAL copy path first, dedup collapses the uniform case.

DECISIONS:
- **Specialization key = the concrete `Vec<Ty>` arg types** (the `Ty` custom `PartialEq` compares
  nominals by decl+args). `render_name` (`ty.rs:1282`) is the human label for the synthesized internal
  def name only.
- **Generalize by NOT grounding an open param to `Any`** in `solve_recursive_params`; instead keep it
  a canonical quantified var and record which vars a def quantifies. Call-site seeding still fires for a
  param the body pins to a CONCRETE-but-underspecified type (numeric width); it stops OVERRIDING a
  genuinely-generic param (one the body only threads).
- **Backstop**: unbounded polymorphic recursion (self-call at an ever-larger type) DECLINES with
  `Code::RecursionBound` — never loops.


## The problem in one sentence

A **non-recursive** generic function already works — it is inlined (β-reduced) at each call site, which
IS monomorphization by the spec's own definition — but a **recursive** generic function is pinned to a
single monomorphic signature by its first use, so a second use at a different type is a hard type error
instead of a second instance.

## Evidence (as-built, verified 2026-07-14)

Non-recursive identity — works, no shared function even emitted:

```
(def (ident v) v)
(def (main (: x Int64) (: y Float64)) (+. (Float64.of-int (ident x)) (ident y)))
```
→ both `ident` calls β-reduce away; `main` is `local.get 0; f64.convert_i64_s; local.get 1; f64.add`.

Recursive polymorphic length — REJECTED at the second type:

```
(def (mylen l) (match l ((List.Nil) 0) ((List.Cons h t) (+ 1 (mylen t)))))
(def (main (: x Int64)) (+ (mylen (list x x x)) (mylen (list "a" "b"))))
```
→ `CDZ0203: type mismatch: Int64 and String must be the same type here` — `mylen`'s element type was
fixed to `Int64` by the first call; the `String` call cannot be represented.

## Why (the mechanism, cited)

- `eval::apply_lambda` **declines** a recursive call (`is_recursive` → `Err`, `eval.rs:710`); it cannot
  β-reduce to a normal form (would inline without end / explode exponentially on a branching body).
- `lower::lower_recursive_call_or_decline` turns that decline into a single `Core::Call { callee: usize }`
  (`core.rs:704`) — the callee is ONE `db.defs` index.
- `layout.rs` emits **exactly one wasm function per reachable def index**; `def_params` (`layout.rs:856`)
  reads `type_of` on each param binder occurrence — so a def has ONE param-type set → ONE function.
- `infer::compute_def_scheme` (`infer.rs:1849`) returns only `Scheme::mono` — the comment is explicit:
  *"A1 schemes are MONOMORPHIC … nothing is quantified. Real let-generalization … arrives with the
  connected solve (A2)."* The `Scheme` struct ALREADY carries `ty_vars`/`width_vars`/`sign_vars`
  (`ty.rs:1377`); they are simply never populated for a def.

The decline messages already name this feature: *"a recursive function needs runtime specialization
(not yet built)"* and glossary's *"replacing a generic definition with concrete specializations by the
same compile-time reduction."*

## The spec says exactly what to build — this is NOT new semantics

- `glossary.md` **Monomorphization**: *"replacing a generic definition with concrete specializations by
  the same compile-time reduction that specializes any definition applied to compile-time-known
  arguments, done before emitting a component interface because generics do not cross the boundary."*
- `component-abi.md §Generics Do Not Cross The Boundary` (MUST): *"The compiler MUST monomorphize every
  exported and imported signature to concrete types before emitting the component interface. A generic
  definition MUST NOT appear in a component's interface."*
- `glossary.md` **Type-valued parameter**: generics are ordinary defs taking types as arguments — "not
  through a separate parametric-polymorphism construct." So there is NO dictionary-passing / no runtime
  type rep to design. Monomorphization is the whole story.

So the target is: **one source recursive def may become SEVERAL emitted wasm functions, one per
concrete instantiation** — the exact thing inlining already does for non-recursive defs, applied to the
one shape that can't inline.

## Does every instantiation need distinct code?

No — and this splits the work into a cheap common case and a general case:

| function | instances | distinct code? | why |
|---|---|---|---|
| `mylen : ∀a. List a → Int64` (counts) | `List Int64`, `List String` | **NO** | element is always an i32 handle; body never touches its rep |
| `member : ∀a. a → List a → Bool` (uses `==`) | `Int64`, `String` | **YES** | `==` lowers to `i64.eq` vs a byte-compare |
| `sum-all` (uses `+`/`+.` on elements) | `Int64`, `Float64` | **YES** | `i64.add` vs `f64.add`; also i64 vs f64 SLOT |

The discriminator is `valtype_of` + which primitive ops the body selects on the type variable. A
representation-uniform body (the type var only ever occupies an i32 handle and is never fed to a
rep-specific primitive) can SHARE one function; anything else needs a copy per concrete type.

## Design

Reuse the machinery that ALREADY makes recursion work across β-copies — `Def::internal` +
`Db::register_reduced_callables` / `modules::register_fn_def` (landed `@8cce52c9`, `@97476002`,
`@0e765cd6`). Those fixes taught the compiler to register a freshly-copied recursive body as an internal
`db.defs` entry so its self-call lowers to a `Core::Call` to the copy. Specialization is the same move,
keyed by concrete instantiation type instead of by inlining site.

### Phase 0 — investigation (decides the phase-1 shortcut's safety)

Confirm before building:
- [ ] `render_name` (`ty.rs:1282`) is injective enough over the ground types a corpus instantiates to
      serve as the specialization key (no two distinct concrete `Ty` render identically). If not, key on
      `encode_ty` bytes instead (`eval.rs`, already a canonical wire form).
- [ ] Enumerate the "representation-uniform" predicate precisely: a def is safe to share ONE function
      across instantiations iff, for every quantified var `v`, (a) `valtype_of` is identical across the
      instantiations at every occurrence of `v` (always i32 handle for a `List a`/`a`-as-handle), AND (b)
      no selected primitive in the body branches on `v`'s type (no `==`/`+`/ordering/`valtype`-sensitive
      op applied to a `v`-typed operand). Decide whether phase 1 ships the shortcut or always copies.
- [ ] Verify a synthesized copy's FRESH param binder occurrences get their own `type_of` memo (they must,
      to carry a different annotated type than the original) — mirrors how `beta_reduce`'s fresh copies
      re-resolve. Cross-check against the `captured_ref`/`should_keep_binding` hazard the do-local fix hit.

### Phase 1 — GENERALIZE the scheme (prerequisite; makes recursive-generic calls TYPE-CHECK)

Make `compute_def_scheme` quantify the vars the A2 connected solve (`solve_recursive_params`) leaves
free after the body walk, instead of grounding/collapsing them:

- After the param + result solve, collect the free `Ty::Var`/`Width::Var`/sign vars remaining in the
  curried signature and put them in `Scheme { ty_vars, width_vars, sign_vars, ty }` (the fields exist).
- A recursive CALL already types by `apply_scheme_to_args` (`infer.rs:2532`), which calls
  `unify::instantiate(scheme, fresh)` — instantiation of a quantified scheme freshens the bound vars, so
  `mylen (list x x x)` and `mylen (list "a" "b")` each get a private instantiation and BOTH type. The
  `CDZ0203` disappears with no call-site change.
- Emission decision for phase 1: for a **representation-uniform** def (per phase 0's predicate), emit ONE
  shared function whose generic slot is the common i32 handle — `mylen`-class works end to end, single
  function, no copy. A non-uniform def still DECLINES cleanly here (unchanged behavior — it becomes a
  Todo, never a miscompile) and is picked up in phase 2. `log`/trace the decline distinctly so the gate
  reads it as "not-yet-specialized," not a regression.

Gate target: `mylen`-class end-to-end (Int64 + String lengths added), plus the existing monomorphic
recursive corpus stays byte-identical (a def with NO free vars generalizes to `Scheme::mono` exactly as
today — additive).

### Phase 2 — SPECIALIZE by copy (the full "instantiate more than once")

For a def whose body is NOT representation-uniform, synthesize a monomorphic type-annotated COPY per
distinct concrete instantiation and route the call to it:

1. At a recursive `Core::Call` whose callee still has free scheme vars, compute the concrete
   instantiation = each arg's `type_of` at THIS call site (already available in `lower`).
2. Memoize on `(callee_def, instantiation-key)` in a new `db.specializations` map. On a miss, synthesize
   `(def NAME$KEY (: p_0 T_0) … <body-copy>) ` — a `beta_reduce`-style structural copy of the body with
   FRESH occurrences and each param binder annotated with its concrete type — and register it via
   `register_fn_def`/`register_reduced_callables` as an internal `db.defs` entry. Its self-call, copied
   the same way, resolves to the copy (identity-preservation exactly as `@0e765cd6`).
3. `Core::Call { callee }` points at the specialization's def index. Because the copy is a monomorphic
   annotated def, `def_scheme`/`def_params` give it a concrete signature → `layout` emits it as its own
   function; `valtype_of` on its now-concrete params is well-defined.
4. **Dedup:** two instantiation keys whose specialized bodies are byte-identical after copy (the
   rep-uniform case that slipped past phase 1's predicate, e.g. two i32-handle element types) collapse to
   one function — hash the emitted core (or the reduced body) and reuse. Keeps `mylen`-at-two-handle-types
   to one function even without the phase-1 shortcut, so phase 1's predicate can stay conservative.
5. `log()` the specialization set per generic def (N instances, M after dedup) so silent per-type
   blow-up is visible — a branching recursive generic could in principle instantiate widely.

Gate target: `member`/`sum-all`-class — a recursive generic whose body selects a rep-sensitive primitive,
called at two concrete types, runs correctly at BOTH (two functions), and a rep-uniform one dedups to one.

### Phase 3 — mutual recursion + nested/module/do-local generics

A mutually-recursive generic group specializes as a group (all members at the connected instantiation),
reusing the group-registration paths (`register_callable`, `register_do_local_callables`). Verify the
module-member and do-local + inlined-helper paths (`@8cce52c9`/`@97476002`/`@0e765cd6`) carry
specialization, since each already registers internal defs.

## Non-goals / explicit boundaries

- NO runtime type representation, NO dictionary passing — generics are compile-time only
  (`glossary` type-valued-parameter). If a build could instantiate an UNBOUNDED family (a
  polymorphic-recursion body that calls itself at an ever-larger type), that is not expressible in the
  monomorphizing model and must DECLINE with a code (the `RecursionBound`/`CDZ0999` class), never loop.
- NO new keys / no name-based special cases (repo rule [[no-keys-outside-the-prelude]]) — the
  specialization key is a TYPE, the synthesized name is internal (never resolved by name).
- Exported signatures stay concrete (`component-abi.md` MUST) — a generic def is never itself an export;
  only its concrete instantiations reach the boundary. An attempt to export a still-generic signature is
  a boundary rejection, unchanged.

## Risks / where this bit before

- **Fresh-occurrence hazards** (`captured_ref`, `should_keep_binding`, `def_by_body` clobber) — the
  do-local + β-copy fixes enumerate these; the copy path here MUST go through the same guarded
  registration, not a bespoke copy.
- **fmt/rebase drift** on `spec` — per the workstream ritual (touch only own hunks).
- **Gate grading** — a not-yet-specialized decline must stay a Todo (codeless or `RecursionBound`), and a
  Todo→Fail flip on any existing case is a miscompile signal, gate the FAIL SET not the count.

## Open questions for sign-off

1. Phase 1 shortcut (share one function for rep-uniform defs) vs. always-copy-then-dedup (phase 2 only)?
   The shortcut ships `mylen` sooner and is byte-smaller, but adds the rep-uniform predicate as a proof
   obligation. Always-copy-then-dedup is simpler to make SOUND (dedup handles the uniform case) at the
   cost of doing the copy work even when unnecessary. Recommendation: build phase 1's generalization
   (needed regardless), but gate emission through phase 2's copy+dedup from the start, and only add the
   rep-uniform single-function shortcut if the emitted-function count proves a problem.
2. Specialization key: `render_name` string vs. `encode_ty` bytes (phase 0 decides).
3. Unbounded-instantiation backstop: reuse the reduction budget / `RecursionBound` code, or a dedicated
   "polymorphic recursion not monomorphizable" code?

## Follow-ons — ALL RESOLVED (each verified + gated)

1. ✅ **Transitive genericity — LANDED** (see the section near the top). A generic-calls-generic chain
   (`wrap→idr`, and the three-level `top→mid→idr`) propagates genericity through the call graph and keeps
   the param↔result var connected, so all functions monomorphize per concrete type.
2. ✅ **Mutual-recursion groups — WORK (no new code).** `ping`/`pong` threading a generic each
   monomorphize at both types; the cross-calls re-resolve by name and re-enter `type_specialize` at the
   same instantiation, so the group specializes as a group without special handling.
3. ✅ **Do-local generics — FIXED** (`copy_structural_pub`'s `pin_self_calls`). A do-local name resolves
   by LEXICAL do-scope, which the specialized copy (re-parented out of the `do` block) escapes → the
   copied self-call re-resolved unbound (CDZ0101). Fix: pin (share) the self-call occurrence so the copy
   keeps resolving it to the original def and re-enters specialization. **Module-member generics** already
   worked (`callee_def_index`'s `Member` arm resolves via `member_value`).
4. ✅ **Unbounded polymorphic recursion — already SAFE.** A body self-calling at an ever-larger type
   (`(bad n x) → (bad (- n 1) (tuple x x))`) is REJECTED at type-check (CDZ0203: the self-call arg type
   conflicts with the param's solved type during the A2 connected solve) and terminates — it never enters
   the specialization loop, so no explicit `RecursionBound` guard is needed. Verified: both a value-
   returning and a discarded-result growing-type recursion reject cleanly (exit 0, no hang).

---

# ADDENDUM: type-valued parameters — a generic def takes the TYPE as a regular argument

Status: DESIGN — direction chosen by the operator (2026-07-14): "use types as values and pass the type
as a regular argument." This is the spec's OWN model (`type-system.md §Generics Are Type-Valued
Parameters`), NOT implicit ML-style type variables. Scope assessment below — this is a MULTI-INCREMENT
vertical, not a single step; the seams are mapped so it can proceed increment by increment.

## The chosen surface
A generic def takes a TYPE-VALUED PARAMETER — an ordinary parameter whose VALUE is a type — and uses it as
a type-constructor argument in its own annotations; the caller passes the concrete type as a normal
argument:
```
(def (unbox (: t Type) (: b (Box t))) (match b ((Box.Mk v) v)))
(unbox Int64  (Box.Mk 40))          ; t = the type-value Int64
(unbox String (Box.Mk "hi"))        ; t = the type-value String
```
`t` resolves by ORDINARY LEXICAL SCOPE (it is just a parameter — no name key, fully honoring
no-keys-outside-the-prelude). This matches the spec verbatim: `type-system.md:224` "a generic definition
MUST be expressed as an ordinary definition that takes type-valued parameters"; `:228` "a generic type
constructor … applied by ordinary application"; `:250` monomorphized before the boundary.

## Spec grounding + the design stance it REVISES
- Type-values are ALREADY first-class in the compiler: `Prim::TypeOf`/`TypeEq` (reflection), a
  `Resolved::TypeVal(t)` value types as `Ty::Type` (`infer.rs:411`), and `(: x (Type.of y))` already
  reuses a reflected type-value in a type position (`prelude.rs:1343`).
- ⚠ The CURRENT deliberate stance (`prelude.rs:1348`): "`Type` in a bare type position is NOT a type (a
  value's type-of is `Ty::Type`, spelled only by reflection)." This feature REVISES that: `Type` must
  become spellable as a PARAMETER annotation so `(: t Type)` declares a type-valued parameter. (An
  operator-blessed change — the spec's type-valued-parameter model requires a way to annotate one.)

## The seams (mapped; each an increment)
1. **`(: t Type)` — accept `Type` as a parameter annotation → the param types `Ty::Type`.**
   `param_annot_ty`→`typeval_of(Type-module)` currently returns `None` (the `Type` prelude record has
   `of`/`eq` fields, no `(meta t)`). Make `typeval_of` map the type-reflection module to `Ty::Type` (the
   kind-of-types), so `(: t Type)` types `t` as `Ty::Type`. (Recognize the module STRUCTURALLY — its
   fields reduce to `Prim::TypeOf`/`TypeEq` — not by the name "Type", to keep it key-free.)
2. **`(Box t)` — a value-param `t : Type` usable as a type-constructor ARGUMENT.** In `reduce_sum_ctor`
   (and the general type-application path), an arg that is a `Resolved::Param` bound to a type-value must
   contribute its type-value. Since `t`'s VALUE is only known at the CALL site, the def's annotation
   `(Box t)` is generic — `(Box ?t)` — until monomorphization substitutes the passed type-value.
3. **Call-site: pass a concrete type as an argument.** `(unbox Int64 …)` — the head `Int64` is a
   type-value argument. `type_specialize` keys on the concrete arg TYPES; here one arg IS a type-value, so
   the specialization is keyed by the passed type-value and the copy substitutes `t := Int64` throughout
   the body (the `(Box t)` annotation, the match). This reuses the existing copy+memo engine; the new
   part is threading a type-VALUE argument (not just a runtime value) into the substitution.
4. **A type-value argument is COMPILE-TIME-ONLY (`type-system.md:226`) — it carries NO runtime slot** and
   must be ERASED from the emitted function's parameters (like a `Ty::Type`/`Unit` param today,
   `lir.rs valtype_of(Ty::Type)=None`). So `unbox`'s emitted arity is 1 (the `Box`), not 2 — the type
   arg is consumed at monomorphization, never passed at run time.

## Increment plan (each gated + landed separately)
- ✅ **T1 — LANDED (`@72a5a8f0`).** `typeval_of` maps the type-reflection module → `Ty::Type`; `(: t
  Type)` accepted (recognized structurally, no name key).
- ✅ **T2 — LANDED (this increment).** In-order SIGNATURE SCOPING: an earlier param is visible in a later
  param's annotation (`binder_in` Case 4b + `def_sig_list_of`/`param_binder_before`, the sig list made a
  `is_binding_candidate`). So `t` in `(Box t)` resolves to the earlier `(: t Type)` param.
- ✅ **T3 (NON-RECURSIVE) — LANDED (this increment).** `typeval_of` reduces a type-valued param in a type
  position to a stable `Ty::Var(binder)` (`type_valued_param_binder`), so `(Box t)` → the generic `(Box
  ?t)`. A NON-recursive `unbox` INLINES at each call site — monomorphization-by-β-reduction, the type arg
  folds away — so `unbox` at Int64+String → 42 with no runtime type arg. The spurious CDZ0306 "unused
  param `t`" (used only in a sibling annotation) is fixed: `used_param_names` now also scans each param's
  annotation type-expression.
- ⏭ **T3 (RECURSIVE) — NEXT.** A RECURSIVE generic with a type-valued param (`(def (len (: t Type) (: l
  (Lst t))) … (len t tl))`) does NOT inline → lowers to a `Core::Call`, and the type-valued arg `t` must
  be ERASED from that call + from the emitted signature (it has no runtime slot, `valtype_of(Ty::Type) =
  None`). TODAY this hits `CDZ0203: Type and Unit` at the recursive self-call (the type arg is threaded as
  a runtime value). FIX: in `lower`'s `Core::Call` + `type_specialize`, DROP a `Ty::Type`-typed argument
  (specialize on its type-VALUE, emit no runtime arg for it). Gate target: `len` over `Lst Int64`+`Lst
  String` with explicit `(: t Type)` → 5.
- **T4** — a type-value arg that is NOT compile-time-resolvable (flows from runtime data) → CDZ0302
  (`:226` "a type-value never flows from runtime data into a position that determines a type").

## Not doing (out of scope, distinct features)
Implicit ML-style quantification `(: l (Lst a))` with a bare `a` (the operator chose type-valued params
INSTEAD); trait/constraint predicates (`:232`); dictionary-passing (`:240`). The UNANNOTATED polymorphic
form already works via inference + monomorphization and is unaffected.
