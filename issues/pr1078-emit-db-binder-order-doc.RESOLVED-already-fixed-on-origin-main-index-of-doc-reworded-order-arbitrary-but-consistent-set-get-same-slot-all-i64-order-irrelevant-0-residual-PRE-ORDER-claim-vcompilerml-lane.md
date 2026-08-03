# PR #1078 review comment — implementation/compiler-ml/src/emit-db.cdz (v-compiler-ml)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1078
(PR: "cand: v-compiler-ml — emit-db.cdz").

## `index-of` doc says PRE-ORDER binders but `collect-binders` produces POST-ORDER (Copilot, emit-db.cdz:204) — correctness/doc
> The new `index-of` docs assert `binders` is a PRE-ORDER binder list, but `collect-binders`
> currently appends the binder at the end of the concatenated child lists via `List.push(...)` (and
> this PR's new test demonstrates `List.push` appends). That makes the binder list POST-ORDER, so
> the local index assignment for nested `CLet`s won't match the stated contract here.

This one matters if nested-`CLet` local index assignment depends on the order: either the doc is
wrong (list is actually post-order) or `collect-binders` needs to prepend/reverse to be pre-order.
Confirm which order the index-assignment logic actually needs and align doc + impl.

---
Dismissed as nit (not filed): duplicate W1 test-section header comment on two consecutive lines
(emit-db.cdz:473, Copilot).
