# PR review comment — mirrored from GitHub PR #404 (Copilot inline)

- **PR:** #404 "fleet: twenty-ninth batch (String.from-bytes runtime fix LANDED, alpha.cdz + slack-bridge recovery, +features)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:2055`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591032988
- **Link:** https://github.com/camshaft/cadenza/pull/404#discussion_r3591032988

## Comment (verbatim)
> `collect_used_ops_into` for `Core::StrFromBytes` claims it needs `arr-alloc` for the None/unit payload and unconditionally inserts `OP_ARR_ALLOC`, but the emitter builds None using the inline-unit constant (`IMM_UNIT`) and never calls `arr-alloc`. This makes the comment misleading and can cause unnecessary runtime imports for programs that only need `str-from-bytes` + `sum-new`.

## Liaison triage — CONFIRMED against trunk
Confirmed in select.rs: the `Core::StrFromBytes` arm of `collect_used_ops_into` does
`out.insert(OP_STR_FROM_BYTES); out.insert(OP_SUM_NEW); out.insert(OP_ARR_ALLOC);`, with a comment
"...build Some/None (sum-new, arr-alloc for None's unit)". But None is built from the inline-unit
constant `IMM_UNIT` (no `arr-alloc`). So `OP_ARR_ALLOC` is declared as a used op unnecessarily — a
program that only needs str-from-bytes + sum-new imports arr-alloc it never calls. Low-severity
(over-declares an import; not incorrect codegen), but it inflates the runtime import set and the comment
is misleading. Used-ops/import-minimization is v-wasm-opt's area. Remove the `OP_ARR_ALLOC` insert (and
fix the comment). Fix on `trunk`. Quote + link in queue file.

## v-wasm-opt resolution (2026-07-15): FIXED in 26dde98c — removed the OP_ARR_ALLOC insert from the Core::StrFromBytes collect_used_ops arm (None uses IMM_UNIT, no alloc). Test str_from_bytes_does_not_over_declare_arr_alloc. Merge-request sent.
