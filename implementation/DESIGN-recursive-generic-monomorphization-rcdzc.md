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

Status: COMPLETE (T1–T4 landed) — a generic definition takes the TYPE as a regular argument, the
operator's chosen model (`type-system.md §Generics Are Type-Valued Parameters`), working end to end for
both non-recursive and recursive callees, with the compile-time-only type argument erased from the
emitted signature/calls, and a type-value that would reach a runtime slot rejected with a coded CDZ0201.
T1 `@72a5a8f0` · T2+T3-nonrec `@064f81c1` · T3-rec `@a37223ca` · T4 (this increment).

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
- ✅ **T3 (RECURSIVE) — LANDED (this increment).** A RECURSIVE generic with a type-valued param
  (`(def (len (: t Type) (: l (Lst t))) … (len t tl))`) lowers to a `Core::Call`, and the type-valued arg
  is now ERASED from both the call and the emitted signature. `type_specialize` classifies each arg: a
  `Ty::Type`-typed arg is a TYPE ARG — its concrete type-VALUE (`typeval_of`) is substituted into the
  copy's body (`(Lst t)` → `(Lst Int64)`, via `copy_structural_pub`'s new `arg_of`), and the param is
  OMITTED from the specialized signature; `lower`'s `Core::Call` drops the type-arg positions from the
  runtime args. So each specialized `len` takes just the list handle (`(i32)->i64`), no type arg. The memo
  key includes each type arg's VALUE (`@Int64`) so distinct instantiations stay distinct. Verified: `len`
  over `Lst Int64`+`Lst String` → 5, two specializations with the type param erased.
- ✅ **T4 — LANDED (this increment).** A type-value can never flow into RUNTIME data — `Ty::Type` has no
  machine representation, so any position that would force it into a runtime slot is rejected. FINDING:
  every compile-time-RESOLVABLE flow (a `let`-bound type, a type threaded through another type-valued
  param) already works (the type-value is statically known → monomorphized), and a NON-resolvable one is
  ALREADY rejected — the guarantee holds structurally. The one gap was DIAGNOSTIC QUALITY: a type-value
  stored in a COMPOUND result (`(def (main) (tuple Int64 5))` : `(Tuple Type Int64)`) leaked the emit
  path's 4-error uncoded no-runtime-form cascade instead of one coded reject. FIX: `Ty::has_type_value`
  (a `Ty::Type`-anywhere walker) + a `collect_faults` arm reporting ONE coded CDZ0201 naming the compound
  (message embeds `TYPE_EXPORT_MARKER` so `dedup_faults` drops the downstream declines), extending the
  bare-type-export CDZ0201 to the nested case. (Note: the guarantee's code is CDZ0201 "not a runtime
  value", NOT CDZ0302 `IntOutOfRange` — the design note misremembered the code; there is no dedicated
  type-determination code, and CDZ0201 is the established "a type is not a runtime value" reject.)

## Not doing (out of scope, distinct features)
Implicit ML-style quantification `(: l (Lst a))` with a bare `a` (the operator chose type-valued params
INSTEAD); trait/constraint predicates (`:232`); dictionary-passing (`:240`). The UNANNOTATED polymorphic
form already works via inference + monomorphization and is unaffected.

---

# ADDENDUM 2: ad-hoc polymorphism via dictionaries — inline a compile-time-known argument, drop the param

Status: LANDED. Operator's framing (2026-07-14): ad-hoc polymorphism needs NO trait machinery — "just
have a record passed as an argument with functions that operate on the data; the function uses the
record; and remove the record argument from the runtime emission when we instantiate it." No global
trait resolution, no orphan rule, no coherence — just compile-time β-folding. The generalized rule the
operator then chose: "we shouldn't specialize on anything [special]; when we instantiate we look for
const-known values and inline them into the function and remove the argument." So `Ty::Type` erasure and
dictionary erasure are the SAME rule — inline any compile-time-known argument, drop its runtime param.

## What already worked (zero new code)
A record of functions passed as an argument, the body projecting `(. d op)` and calling it, works for
BOTH non-recursive and recursive consumers — it is just records + functions + application. A
NON-recursive consumer already fully ERASES the dict (β-fold inlines it away). A RECURSIVE consumer
WORKED but kept the dict as a runtime heap record + `call_indirect` (a closure in the record), threaded
through the recursion — correct, but unerased.

## What this landed — recursive dictionary ERASURE
`lower_recursive_call_or_decline` now specializes a recursive call when EITHER the callee scheme is
generic (a type param) OR an argument is a compile-time-known DICTIONARY. `type_specialize` gained a
third `ArgKind::ConstArg`: the dictionary's VALUE NODE is substituted into the specialized copy (so `(. d
op) acc` β-folds to the concrete `(+ acc 10)` — no `call_indirect`, no runtime record) and the parameter
is ERASED from the emitted signature. The self-call re-passes the (copied) dict, re-enters
`type_specialize`, and hits the memo (keyed on a `subtree_fingerprint` — stable across the arg and its
β-copy, distinguishing `+10` from `*2` while collapsing identical dicts). Verified: `fold-n` at one dict
→ 30 with **0 `call_indirect`** and signature `(i64 i64)` (dict gone); two distinct dicts → 38, each op
inlined; two identical dicts → one deduped function.

## The scope guard (a regression caught + fixed)
The predicate `arg_is_const_inlinable` fires ONLY for a record/tuple that CONTAINS A LAMBDA (a
dictionary), and only when CLOSED (no field-lambda captures the consumer's own param / a runtime
binding). It must NOT fire for a pure-DATA collection: a first pass inlined a const `(list 10 20 30)`
argument to a recursive `sum`, unfolding the recursion at compile time without end → stack exhaustion
(one gate FAIL). A data list/record is RUNTIME data whose per-call value drives the base case; only a
value whose CONTENT is functions is worth (and safe to) inline. Requiring a lambda restricts erasure to
the dictionary case; data flows as ordinary runtime args. Non-lambda dict fields must be closed constants
(config alongside the ops). Conservative everywhere else → the value stays a runtime arg (the correct,
pre-existing `call_indirect` path).

Gate 1850/0, 1318 rcdzc tests + the new dictionary test (asserts no `call_indirect` via `wasmparser`),
+2 corpus cases (09-functions).

---

# ADDENDUM 3: EXPLICIT `const` parameters — the author declares what is compile-time

Status: DESIGN → implementing. Operator's direction (2026-07-14): the IMPLICIT dict-sniffing of Addendum
2 is "overly specific and inflexible" — the compiler guessing "is this arg a const dictionary" is what
produced the const-data-list stack-overflow footgun. Replace it with an EXPLICIT `const` parameter: the
FUNCTION declares which parameters are compile-time-known; the compiler obeys (inlines + erases them) and
ERRORS if a `const` argument cannot fold. No detection heuristic, no specialization-guessing.

## Surface (operator's choice)
A parameter binder is wrapped `(const BINDER)` to mark it compile-time:
```
(def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
  (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc))))
(fold-n (record (op (fn (x) (+ x 10)))) 3 0)   ; d is const → inlined + erased; n/acc runtime
```
- A `const` parameter MUST be compile-time-known at each call site: it is inlined into a specialized copy
  (so `(. d op)` folds to the concrete op — no `call_indirect`, no runtime record) and ERASED from the
  emitted signature. `n`/`acc` stay ordinary runtime parameters.
- A non-foldable argument to a `const` param is a CODED COMPILE ERROR ("argument to const parameter must
  be compile-time-known"), NOT a silent runtime fallback — the author declared the contract.
- This SUBSUMES the type-valued parameter: `(: t Type)` is just a `const` param whose value is a type.
  (Kept working for back-compat; the general `const` is the primary surface.)

## Why this is cleaner than Addendum 2's heuristic
- The trigger is a DECLARATION, not a guess. No `arg_is_const_inlinable` sniffing "is this a record of
  lambdas"; no accidental inlining of a const data list (the stack-overflow footgun) — a data arg is
  const only if the AUTHOR marks it, and then it must genuinely fold or it is an error.
- More flexible: ANY value can be a const param (a config record, a comparator, a type, a tuning
  constant), not just "a record containing a lambda".
- One rule everywhere: at instantiation, a `const` param's argument is folded in and dropped.

## Implementation — load-time strip + a const-param set (mirrors `strip_def_docs`)
1. **`strip_const_params(&mut ast) -> FxHashSet<StructId>`** in `db.rs`, run in `Db::load` BEFORE
   `scan_top_level` (exactly like `strip_def_docs`): for every `def`/`fn` signature, rewrite each
   `(const BINDER)` child in place to `BINDER`, and record the stripped param's NAME occurrence
   (`param_name_occ` of the inner binder) in the returned set. After this pass every downstream reader
   (`param_name_occ`, `is_param_occurrence`, the 12 `(: name T)` unwraps, resolve, infer) sees a PLAIN
   binder — ZERO changes to any of them. `const`-ness lives in `db.const_params`.
2. **`type_specialize`** (`lower.rs`): DELETE the `arg_is_const_inlinable` heuristic + the `Ty::Type`
   auto-detection. Classify a param as erased iff it is in `db.const_params` (or its solved type is
   `Ty::Type` — the type-valued back-compat case). For a const param, fold its argument to a value
   (`typeval_of` for a type, else the arg's value node) and substitute it into the copy; require the fold
   to succeed or the call declines with the coded error. Same substitute-into-copy + drop-from-signature
   machinery, now keyed on the DECLARATION.
3. **The gate** in `lower_recursive_call_or_decline`: specialize when the scheme is generic OR the callee
   has any `const` param (`db.const_params` intersects the callee's params). A non-const monomorphic call
   is byte-identical to today.
4. **The coded error**: a `const` param whose argument does not fold (references runtime data) →
   `Code::Malformed` (CDZ0201) "argument to const parameter `d` must be compile-time-known".
5. `const` joins the resolver `GRAMMAR` set only if a bare `const` name could otherwise be misread — but
   since the strip runs at load before resolution and removes every `(const …)` wrapper, `const` never
   reaches resolution as a head. (A `const` used as an ordinary NAME elsewhere is unaffected.)

## Non-recursive const params
A non-recursive call already β-reduces (inlines) the whole body at the call site, so a const param is
folded away for free there too — the const marking additionally lets the compiler REJECT a non-foldable
argument (the contract) rather than silently keeping it. The recursive case is where erasure needs the
specialized copy (as before).

## ML surface (both sides)
`const` is a param modifier on BOTH surfaces. S-expr: `(const (: d T))` / `(const d)`. ML: `const d: T` /
`const d` (the printer emits `const ` before the binder; the parser's `param` accepts a leading `const`
identifier followed by a binder, wrapping it `(const …)`). `const` is NOT a lexer keyword — a bare param
literally named `const` (with no following binder) stays an ordinary name. Verified: the corpus dict cases
round-trip s-expr↔ML↔binary cleanly.

## Landed
Gate 1890/0; corpus: the two dict cases now use `(const (: d …))` + a new "a const parameter rejects an
argument that depends on runtime data" (CDZ0201); +1 unit test (the const-contract reject) + the dict
erasure test updated to `const`. The implicit dict-sniffing heuristic (`arg_is_const_inlinable` and its
helpers) is DELETED — erasure is now driven purely by the `const` DECLARATION (`db.const_params`, filled
by `strip_const_params` at load), which also subsumes the `Ty::Type` type-valued-parameter case. Soundness:
`arg_captures_runtime_binding` rejects a `const` arg that captures ANY enclosing runtime param (not just
the callee's own), closing the caller-capture gap the `own`-only check missed.

---

# ADDENDUM 4: INLINE POLICY — `inline-never` / `inline-always`, default always-inline, heuristic deferred

Status: LANDED (`inline-never` fully; `inline-always` recorded + conflict-rejected, inert until the
heuristic; cost heuristic still deferred). Operator's concern (2026-07-14): the compiler inlines EVERY
non-recursive call unconditionally, so a helper called N times emits its body N times (verified: a 5-mul
helper called 3× → 15 muls). Wanted: author control over inlining, in the Rust
`inline`/`inline(never)`/`inline(always)` spirit. AS BUILT: `strip_inline_policy` load pass →
`db.inline_never`/`db.inline_always`; `lower.rs` routes an `inline_never` call to the shared
`emit_call_or_specialize` (factored out of `lower_recursive_call_or_decline`) so it emits-once-and-calls
AND still specializes a generic/`const` callee; `compile.rs` rejects `inline-always` on a recursive def
(CDZ0201); `cadenza-syntax` printer+parser do `inline-never`/`inline-always def …` (round-trips). Verified:
`inline-never big` ×2 → 3 muls not 6; `inline-never`+`const` dict → 0 `call_indirect`, one fn per distinct
dict; `inline-always` on recursion → CDZ0201.

## Why Cadenza is NOT Rust here (this reframes the whole feature)
In Rust, codegen emits CALLS and inlining is an OPTIMIZATION on top. In Cadenza, the compiler LOWERS BY
β-reduction — inlining IS the fundamental lowering, and a `Core::Call` is the FALLBACK it is forced into
only when it cannot inline (recursion). So the default is already maximal inlining, which inverts the
three Rust knobs:
- **`inline(always)`** = the CURRENT DEFAULT — every non-recursive call already does this.
- **`inline(never)`** = the genuinely new lever — force a `Core::Call` instead of β-reducing (the `opaque`
  idea). The one knob that adds a capability TODAY.
- **`inline` (hint)** = ~meaningless — the default is already maximal inlining.

## DECISION (operator, 2026-07-14): two explicit markers now, cost heuristic LATER
- **Default (unannotated) = ALWAYS INLINE**, exactly as today. Kept because during the compiler-port-to-
  Cadenza work, always-inline gives fully-specialized, call-free output that is trivial to reason about
  and diff, and a component's content hash does NOT depend on any inliner-threshold tuning.
- **`inline-never`** — emit as ONE real wasm function; every call is a `Core::Call`, never β-reduced. The
  real lever (the former `opaque`). Always correct.
- **`inline-always`** — explicit "always fold me." A NO-OP vs. the default TODAY, but forward-compatible:
  the day the heuristic lands it becomes the override meaning "ignore the cost model, always inline."
- **COST HEURISTIC — DEFERRED.** A cost-based default (inline small/few-use, emit-call for big/many-use)
  is appealing for code size but (a) flips the default codegen strategy, (b) makes emitted bytes depend on
  threshold constants, and (c) needs the MANDATORY-INLINE invariant below. Right thresholds come from real
  code-size data the self-hosted compiler will produce — tune it THEN, as a separate measured change. When
  it lands, the unannotated default becomes "heuristic", and `inline-always`/`inline-never` are its overrides.

## 🚧 The invariant a future heuristic MUST respect: inlining is MANDATORY when the result is demanded
A cost heuristic can NOT be "cost over everything." Inlining is REQUIRED, not optional, whenever a call's
result is needed at COMPILE TIME: a `const` argument, a type-valued position, a generic instantiation, or
ordinary constant folding (`(+ (double 3) 1)` → `7` only because `double` inlines). If the heuristic emitted
such a call as a runtime `Core::Call`, the "must be compile-time-known" contract (Addendum 3) breaks. So
the rule is: **inline when the result is compile-time-DEMANDED (mandatory); otherwise a cost heuristic
picks inline-vs-emit for an ORDINARY runtime call.** `inline-never` on a compile-time-demanded call is
therefore itself a conflict → a coded reject ("this call's result is needed at compile time; it cannot be
`inline-never`"), NOT a silent miscompile.

## THE KEY INTENT (operator, 2026-07-14): `inline-never` COMPOSES WITH `const`/generics
`inline-never` must be "avoid the inline but STILL get polymorphism." It is orthogonal to `const`/generic
monomorphization:
- `const`/generic decides HOW a call is SPECIALIZED (a const dict / type erased into a per-instantiation
  copy — the polymorphism).
- `inline-never` decides how the (specialized) callee is EMITTED — as one real function, called.
So an `inline-never` def with a `const` dictionary parameter: the dict is STILL inlined into the
specialized copy (polymorphism kept — direct op, no `call_indirect`, dict erased from the signature), and
that copy is emitted ONCE and `Core::Call`ed at every site of that instantiation (inline avoided).
Monomorphic polymorphism WITHOUT per-call-site body duplication. This falls out for free (see below).

## The surface — def-level inline-policy markers
```
(inline-never  (def (big x) …))          ; s-expr           opaque-never def big(x) = …   ; ML: `inline-never def`
(inline-always (def (big x) …))          ;                  inline-always def big(x) = …
```
Def-level markers (chosen over a call-site `(inline-never (big a))` or a full optimization barrier): the
common need is per-def, local, and SYMMETRIC with `const`. `inline-never` is code-LAYOUT control
(emit-once), NOT an analysis barrier — the compiler still reasons about `big`'s type/result + specializes
its const/generic args; it just does not duplicate the residual body. (A full black-box-to-analysis
barrier is a heavier, separate feature; deferred.)

## Why it's small — it REUSES the recursive-call emission path
The `Core::Call` machinery already exists for recursion: `lower_recursive_call_or_decline` emits a
`Core::Call { callee }`, `layout` reaches the callee and emits it once, `def_scheme` gives the callee its
signature. An `inline-never` NON-recursive def rides the SAME path. Seams (mirroring `const`/`strip_def_docs`):
1. **`db.rs strip_inline_policy`** — a load pass (like `strip_const_params`) unwraps `(inline-never (def …))`
   / `(inline-always (def …))` → `(def …)` in place and records the def's body occ in `db.inline_never:
   FxHashSet<StructId>` (and, once the heuristic exists, `db.inline_always`). Every downstream reader sees a
   plain `(def …)`; the policy lives only in the set(s). `inline-always` is recorded but INERT until the
   heuristic lands (the default already always-inlines).
2. **`lower.rs` the apply path (~943)** — BEFORE β-reducing a lambda head, if the callee's body is in
   `db.inline_never` (via `callee_def_index`), route to the SAME code `lower_recursive_call_or_decline`
   runs: it (a) reads `def_scheme(callee)` (an `inline-never` def needs a determined signature, like a
   recursive one; undetermined → the "annotate its parameters" decline), and (b) if the scheme is GENERIC
   or the callee has a `const` param, calls `type_specialize` — which erases the const dict/type into a
   per-instantiation copy and returns a `Core::Call` to it; else a plain `Core::Call { callee }`. 🎯 THIS
   IS WHERE `inline-never` + `const`/generics COMPOSES FOR FREE: routing the call through the recursive
   emit path means an `inline-never` GENERIC or `const`-dict def is specialized (polymorphism + dict
   erasure kept) AND emitted once + called (inline avoided) — no extra logic, the existing `type_specialize`
   branch does both. Factor the generic/const/plain decision out of `lower_recursive_call_or_decline` into
   a shared `emit_call_or_specialize(callee, args)` both the recursion decline and the `inline-never` path
   call. ⚠ MANDATORY-INLINE guard: if the call's RESULT is compile-time-demanded (feeds a `const` param /
   type position / a constant fold) an `inline-never` def is a conflict → a coded reject (see the invariant
   above), not a silent runtime call.
3. **`layout`** — no change: a `Core::Call` to the callee already grows the reachable set + emits it once.
   `def_params`/`select_function` emit its body once with its solved param types.
4. **`cadenza-syntax` printer + parser** — emit/accept `inline-never` / `inline-always` before a `def`
   (mirrors the `const` param work): the printer prefixes the keyword, the parser accepts a leading
   `inline-never`/`inline-always` on a def. Round-trip both. (These are hyphenated identifiers, not lexer
   keywords, like `const` — a bare def named that stays a name.)

## Interactions to pin at build time
- **`const` + `inline-never` — THE HEADLINE (operator: "avoid the inline but still get polymorphism")** —
  a `const` dictionary param on an `inline-never` def: the dict is STILL inlined into the specialized copy
  (so `(. d op)` folds to a direct op — polymorphism kept, dict erased from the signature), and that copy
  is emitted ONCE per instantiation and `Core::Call`ed everywhere it is used at that instantiation (inline
  avoided). Monomorphic polymorphism without per-call-site body duplication — exactly the goal. One
  emitted fn per DISTINCT const instantiation (the `type_specialize` memo already dedups); calls at the
  same instantiation share it.
- **Generics** — an `inline-never` GENERIC def (a free scheme var) still monomorphizes per type — one real
  emitted function per concrete type, called (not inlined) at each use. `inline-never` stops the *inline*,
  not the *specialize*.
- **A NULLARY `inline-never` def** — today a nullary non-exported def inlines at its (one) call;
  `inline-never` keeps it a function. Low value but consistent.
- **Effects** — an `inline-never` def that performs an effect still needs its handler in scope; the policy
  is about emission, not effect routing. Likely a clean decline if it complicates the handler
  specialization (`effects::specialize_recursive` already emits real functions).
- **`inline-always`** — today a NO-OP (the default already inlines); recorded for when the heuristic lands,
  where it becomes "ignore the cost model, always fold." An `inline-always` on a RECURSIVE def is a
  conflict (it can't inline) → a coded reject.

## Gate targets (when built)
- `(inline-never (def (big x) …))` called 3× → the module has ONE `big` function + 3 `Core::Call`s (5
  muls, not 15); an un-marked `big` stays fully inlined (byte-identical to today).
- 🎯 `(inline-never (def (fold-n (const (: d …)) n acc) …))` called at ONE const dict from several sites →
  ONE emitted `fold-n#mono` with the dict INLINED (0 `call_indirect`, dict-less signature) + a `Core::Call`
  at each site (not N inlined copies). At TWO distinct dicts → two emitted specializations. This is the
  "avoid the inline but keep polymorphism" acceptance test.
- `inline-never` round-trips s-expr↔ML; an `inline-never` generic still monomorphizes per type.
- `inline-never` on a compile-time-demanded call (result feeds a `const` param) → coded reject; on a
  recursive def → no-op (already emitted once).
- `inline-always` parses + round-trips and is a NO-OP vs. the default (byte-identical) until the heuristic.

## FUTURE: the cost heuristic (a separate, measured change — NOT this addendum)
When the self-hosted compiler gives real code-size data, add a cost-based default for the UNANNOTATED,
NON-compile-time-demanded, non-recursive call: inline when cheap (small body, or few call sites — roughly
`body_size × (call_sites − 1) < threshold`), else route to `emit_call_or_specialize` (a real function).
At that point: unannotated default = heuristic; `inline-always` overrides it to always-fold; `inline-never`
overrides it to always-call. The MANDATORY-INLINE invariant above is unconditional — the heuristic only
ever chooses for a call whose result is NOT compile-time-demanded. Tuning the threshold is the measured
part; the mechanism (route the not-inlined case through `emit_call_or_specialize`) already exists.
