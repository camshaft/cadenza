# PR #1838 review comments — rcdzc/src/{compile,infer}.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1838 (reject a wrong-arity generic in a variant-payload — my
#1832 wrong-arity lineage). One remaining hole + a doc.

## 1. BARE generic ctor (missing all args) still accepted at payload/op-type positions — inconsistent with annotations (Copilot, compile.rs:1119) — correctness [VERIFIED]
> `validate_type_position` now rejects over-/under-arity APPLICATIONS via `type_ctor_arity_message`, but
> still silently accepts a BARE generic type constructor name (missing arguments) in payload/op-type
> positions (e.g. `(type W (Wrap Box))` / `(Wrap Option)`) — `typeval_of` succeeds on the bare ctor and
> the function returns early. Annotation positions reject this (`bare_type_ctor_needs_argument`). Remaining
> wrong-arity hole at declaration sites.
VERIFIED: validate_type_position (compile.rs:1119) checks `type_ctor_arity_message` (applied ctors like
`(Box Int64 Bool)`) then `if typeval_of(pos).is_some() { return }`. A BARE ctor (`Box` with no args) isn't
an application, so type_ctor_arity_message doesn't fire, and `typeval_of` SUCCEEDS on it (reduces to the
sum silently) → waved through. But the ANNOTATION path catches exactly this via
`bare_type_ctor_needs_argument` (infer.rs:1801/2501). So `(Wrap Box)` at a payload/op-type DECLARATION is
accepted while the same shape is rejected in an annotation — an inconsistency + a wrong-arity hole that
resurfaces later as a confusing CDZ0201 at construction. Fix: also call `bare_type_ctor_needs_argument` in
validate_type_position (before/alongside type_ctor_arity_message). MED — completes the wrong-arity fence at
declaration sites. Fix-forward.

## 2. `type_ctor_arity_message` doc now misleading — says "annotation" + "prelude" only (Copilot, infer.rs:2157) — doc
> The doc for `type_ctor_arity_message` says it only applies to "annotation" and "prelude type
> constructors", but the impl is now also used by validate_type_position (payload/op-type positions).
Update the doc to reflect it's used at payload/op-type positions too (not just annotations). LOW/doc.
