# PR review comment — mirrored from GitHub PR #388 (Copilot inline)

- **PR:** #388 "fleet: fifteenth batch (call_indirect fix, closure-dispatch fix, iterator overflow fix, corpus + fuzzer oracle)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:4527`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590046340
- **Link:** https://github.com/camshaft/cadenza/pull/388#discussion_r3590046340

## Comment (verbatim)
> `closure_type_index` currently rejects (returns `None`) for closures whose full application result is `Unit`, because it uses `valtype_of(&result_ty)?`. However the serializer (`functype` / `closure_call_functype`) and the new `Layout::closure_call_types` docs explicitly treat `Unit` as a valid zero-result functype. This inconsistency can still cause valid `call_indirect` sites returning `Unit` to decline with "a runtime closure application has no matching function type" even when a matching lifted lambda (or extra closure-call functype) exists.
>
> Treat `Unit` as a distinct, representable return shape (zero results) when matching lifted lambdas and `closure_call_types`.

## Liaison triage — CONFIRMED against trunk
Confirmed: in `select.rs` `closure_type_index`, `let rv = valtype_of(&result_ty)?;` short-circuits to
`None` for a Unit result, because `lir.rs valtype_of` has `Ty::Unit => None`. But the serializer builds
`closure_call_functype`s that treat Unit as a valid ZERO-result functype. So a runtime closure whose
full application yields `Unit` can decline ("no matching function type") even when a matching lifted
lambda / extra closure-call functype exists — an eager decline, potentially a missed valid
`call_indirect`. This is closure-dispatch/wasm-lowering correctness, and it neighbors the fleet's
tracked closures-across-host-boundary work + the Unit-param-closure-boxed gap. Route to `corpus-bugfix`
PM to verify against a reproducer (a HOF applied to a `Unit`-returning closure) and, if real, match the
zero-result shape rather than declining. Fix on `trunk`. Quote + link in queue file.
