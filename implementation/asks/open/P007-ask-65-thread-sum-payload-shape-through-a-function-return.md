## 65. Thread a sum/tuple PAYLOAD SHAPE through a function RETURN — the accessor-on-a-returned-payload gap

**Status: 🔴 OPEN (sibling's HOL Light kernel blocker). A real shape-inference gap, a dedicated pass.**

**Symptom (two corpus FAILs, 05-compound-types.sexp).** A helper returns a sum payload; the CALLER then
projects it:
- `(def (unbox bx) (match bx ((Box.B t) t)))` then `(tuple.1 (unbox (Box.B (tuple (list) (Term.Var
  7)))))` — with a `let`-bind of the result — is REJECTED CDZ0201 "tuple access on a non-tuple" (a valid
  program asserted ill-typed). §"a tuple payload extracted through a helper return must not be rejected".
- `(def (get o) (match o ((Some p) p) (None (tuple 0 0))))` then `(tuple.0 (get (Some (tuple 7 8))))`
  emits a VALID component that TRAPS at run time (WORSE — emit-a-broken-component). §"a tuple payload
  returned through a helper from a built-in Option must not trap".
Both INLINE controls PASS (`(match (Some (tuple 7 8)) ((Some p) (tuple.0 p)) …)` → 7): **the payload's
shape IS available where the binder is bound; the gap is threading it through a bare function RETURN.**

**Root cause.** `Func` tracks `ret_kind` but NOT a `ret_shape`. `shape_of` DOES inline a user call
(binds params to arg nodes, shapes the body) — so for a call with a CONCRETE argument it can often
recover the shape — but (a) the emit/`gen_tuple_access` path and the `check_type_rejections` reject
path do not consistently consult that inlined shape for a `let`-bound call result, and (b) a helper
matched on a PARAMETER of a sum type (the general case: `unbox`/`get` called with a non-constant arg)
has no param-type → binder-shape → ret-shape threading at all. The match-binder `t`/`p` should take the
DECLARED payload shape of the matched variant (`sum_payload_types` already holds it), and the function's
return shape should be that binder's shape, recorded on `Func` and consulted at every call site.

**Fix direction (a dedicated pass, non-trivial).**
1. Add `ret_shape: Option<Shape>` to `Func`; compute it during/after `infer_kinds` for a body whose
   value is a match-bound payload (bind each `(Ctor binder)` arm's binder to the variant's declared
   `sum_payload_types` shape, then shape the arm body; unify across arms like the existing `match`
   shape rule).
2. `shape_of` of a user-function call returns `f.ret_shape` (already inlines when the arg is concrete;
   this covers the parameter case too).
3. Ensure `gen_tuple_access` / `gen_member` on a `let`-bound call result read that shape (via the
   local's stored `Shape`, already threaded by `gen_let` from `shape_of`), and that the
   `check_type_rejections` `tuple.`/`.` reject does NOT fire when the operand's shape is a known
   tuple/record reached through a return.
4. Until threaded, the built-in `Option` case MUST at least DECLINE (not emit a trapping component) —
   a stopgap honoring decline-don't-miscompile even before the full shape threading lands.

**Acceptance.** Both cases yield their VALUES (1 and 7); the inline controls stay green; no regression.
This unblocks a HOL `concl`/`dest_thm`-style accessor that returns a payload term for the caller to
consume (the sibling's LCF kernel spike surfaced exactly this: "payload shape recovered only INLINE-in-
arm, not thru fn-return"). Related: [[runtime-record-field-access-and-payload-shape]],
[[nested-tuple-binder-in-sum-payload]], [[hol-light-kernel-spike-2026-07-08]], task #33 (fn returns heap
value from a match binder — the kind twin of this shape gap).
