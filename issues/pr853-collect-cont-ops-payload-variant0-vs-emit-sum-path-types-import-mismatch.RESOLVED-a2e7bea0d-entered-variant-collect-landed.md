# PR#853 review comment — collect_cont_ops_rec resolves Payload via variant-0, but emit uses sum_path_types → import-set mismatch on multi-variant sums

Mirrored from GitHub PR review comment (Copilot), id `3648027920`.
PR: https://github.com/camshaft/cadenza/pull/853 (merged; fix belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:3543` (`collect_cont_ops_rec`, the LitTest import walk).

## Comment (verbatim)

> `collect_cont_ops_rec`'s LitTest import collection re-walks `path` using variant-0 for `Payload`
> steps, but emit resolves `Payload` through recorded entered-variant types (`sum_path_types`). For
> multi-variant sums, this can misclassify a payload as non-BigInt and omit BigInt-related imports,
> leading to wrong `CallImport` indexing or missing imports at runtime.

## Liaison verification (CONFIRMED on trunk — potential miscompile)

select.rs:3527-3533: the import-collection walk resolves a `Payload` step as
```
Ty::Sum { .. } => { lit_cur = variant_payload_ty_at(db, &lit_cur, 0).unwrap_or(Ty::Any); }
```
— hardcoded VARIANT 0. Its own comment (3520-3523) admits: "variant-0 fallback, matching emit's
no-`sum_path_types` case — exact for a SINGLE-variant sum". But the actual emit (`emit_littest_probe`)
resolves `Payload` through the recorded ENTERED-variant type via `sum_path_types`. The comment right
below (3517-3519) states the invariant: "MUST agree with `emit_littest_probe`'s `cur` so the import
set matches the emitted ops exactly — an extra/missing import shifts every `CallImport` index."

For a MULTI-variant sum where the entered variant ≠ 0 and the BigInt-ness of variant-0's payload
differs from the entered variant's (e.g. `(type T (A BigInt) (B Int64))` matched on a `B` payload, or
vice-versa): the import walk classifies by variant-0's payload while emit classifies by the entered
variant → the import set (OP_BIGINT_CMP/OF_I64/… vs get-int) diverges from the emitted ops → every
subsequent `CallImport` index shifts → wrong-op import call or a missing import at runtime. This is a
latent miscompile (guarded today because the BigInt-sum-payload littest cases are likely single-variant
newtypes `(type W (Mk BigInt))`, which is exactly the variant-0 case — but the code is unsound for the
multi-variant shape the emit path already handles).

Fix: make the import walk resolve `Payload` through the SAME `sum_path_types`-recorded entered variant
that `emit_littest_probe` uses (thread the same scrutinee-keyed path types), not a variant-0 fallback —
so the two can never diverge. Owner: v-inference (emit-type-selection / `sum_path_types` lane; BigInt
sum-payload littest landed `5505b5010`; sibling of the PR#743/#769 select.rs items routed there).
Routed as a note flagged POTENTIAL-MISCOMPILE (import-index shift on multi-variant BigInt-payload sums).
Repro attempt: a `(match … )` littest over a 2-variant sum whose entered variant's payload is BigInt
but variant-0's is a fixnum (or vice-versa), inside a continuation-op collection path; emit + run,
check the CallImport indices resolve.
