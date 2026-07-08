# The annotation payload check descends one level and skips list elements

*2026-07-08*

**What happened.** After the one-level annotation-payload check landed (cycle 2:
`(: (Some true) (Option Int64))` rejects CDZ0203), adversarial probing found two cases it still
lets through, both wrong-accepts of ill-typed programs:

1. **Nested payload, depth 2.** `(: (Some (Some 5)) (Option (Option Bool)))` runs to `(Some (Some
   5))`. The value is `Option (Option Int64)`; the annotation is `Option (Option Bool)`; both
   `Option` heads agree but the innermost `Int64` cannot unify with `Bool`. The one-level check
   descends into the outer parameter but compares the nested payload only by coarse kind (both are
   a sum / `Option`), so it never reaches the innermost `Bool` and accepts.
2. **List element type.** `(: (list 1 2) (List Bool))` runs to `(list 1 2)`, and `(: (list true)
   (List Int64))` likewise. The annotation check never validates a list's element type against
   `(List T)` at all.

**Why they are breaks.** type-system.md #Annotations Constrain, Never Contradict: "A program whose
annotation cannot be unified with the type inference determines MUST be rejected." Both are
unification failures — `Option Int64 ≠ Option Bool` at depth 2, `List Int64 ≠ List Bool` at the
element — so both must reject (CDZ0203), exactly as the one-level Option case does. Accepting them
runs a program the spec says is ill-typed.

**Root cause — the descent is one level deep and uses coarse kind at the leaf.** In the seed
(`codegen.rs`, the `":"` arm of `check_type_rejections`), the payload check is: for a
constructor-application value under a parameterized annotation, take the annotation's payload type
*name* (`annotation_payload_param`) and compare `static_type(payload)` against it via
`matches_annotation`. That works one level: `Some`'s payload `true` has `static_type == Bool`,
compared to the name `Int64`, mismatch. But for `(Some (Some 5))` the payload is `(Some 5)` whose
`static_type` is the coarse kind `Sum` and whose annotation parameter is the *compound*
`(Option Bool)` — `matches_annotation` compares the kind `Sum` against the head name `Option`,
which agrees, and there is no recursion into `Bool`. The code comment even states the intent —
"a nested/compound parameter is not descended (decline-don't-miscompile)" — but the fall-through
*accepts* rather than *declines*, the same unknown⇒accept inversion the cycle-4 constructor break
had. And no arm descends into a list's element type.

**The lesson (the annotation check joins the recurring family).** A parameterized-type contradiction
can occur at any depth and in any type constructor (sum payload, list element, tuple element, map
value). The one-level fix closed the shallowest, most common case, but "descend into the parameter"
has to mean *recurse structurally through the full annotation and the full value shape*, not
"descend once and compare kinds." The durable fix is a single recursive `type_contradicts(value_shape,
annotation_node)` that walks both in lockstep — sum⇒match variant then recurse payload vs parameter;
list⇒recurse element vs `(List T)` parameter; tuple⇒recurse each element vs `(Tuple …)` parameters;
leaf⇒`matches_annotation` — and, crucially, returns *decline* (not accept) for a parameter it cannot
yet judge, so an un-recursed case is never a silent accept. This mirrors the pattern-shape fix that
closed the same shape for match arms (2026-07-07): one structural walk, per-node dispatch, unknown⇒
decline.

**Corpus cases added.** `spec/semantics/07-type-system.sexp` §"a nested option value annotated with the
wrong inner payload type is rejected" (`(: (Some (Some 5)) (Option (Option Bool)))`) and §"a list
annotated with the wrong element type is rejected" (`(: (list 1 2) (List Bool))`), both MUST reject
CDZ0203, as the recursion companions of the one-level option-payload case. Native seed; the behavior
gate catches both (expected reject CDZ0203, observed a running component).

**Also observed, NOT pinned (a false decline, lower priority).** `(: (Ok 5) (Result Int64 Bool))` and
`(: (tuple 1 2) (Tuple Int64 Int64))` — both well-typed — decline "over-applying a single-arity
constructor." The two-parameter type constructors `(Result A B)` / `(Tuple …)` in annotation position
confuse an arity check into treating the annotated value as over-applied. A decline is graded todo (not
a FAIL / not a miscompile), so this is a false-reject to fix alongside, not pinned as a gate case.
