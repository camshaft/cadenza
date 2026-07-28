# PR#815 review comment — s3_result_ok Record-arm comment claims rust_call_arg needs a sorted-field fix, but it already sorts

Mirrored from GitHub PR review comment (Copilot), id `3635068299`.
PR: https://github.com/camshaft/cadenza/pull/815 (merged; fix belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/backend/rust/mod.rs:281`

## Comment (verbatim)

> The comment says the record ARG side is blocked on a sorted-field-order fix in `rust_call_arg`, but
> `cdz_rust_render::rust_call_arg` already sorts named record pairs into canonical key order (see xtask
> tests). This makes the justification misleading; either update the comment to reflect the real
> remaining blocker (if any), or drop the claim about `rust_call_arg` needing a fix.

## Liaison verification (CONFIRMED on trunk)

- mod.rs ~278-281 (`s3_result_ok` Record arm, added `99a70ca02`) says: "(The record ARG side —
  `s2_arg_ok` — is NOT yet widened for Record: the harness's `rust_call_arg` record-literal→positional-
  tuple rebuild needs a sorted-field-order fix first, or the arg mis-marshals — a separate follow-up
  slice.)"
- But `cdz-rust-render/src/lib.rs` `rust_call_arg` (the `"record"` case, ~205-226) ALREADY sorts named
  pairs: `fields.sort_by(|a, b| a.0.cmp(&b.0));` with the comment "sort by NAME so the positional tuple
  matches the backend's sorted-key field order." So the claimed prerequisite ("needs a sorted-field-
  order fix first") is already satisfied — the justification is stale.

So the record-ARG-side deferral is either (a) actually unblocked on the sort front (the real remaining
blocker, if any, is something else — e.g. `s2_arg_ok` simply hasn't added the `Ty::Record` arm yet), or
(b) the comment should drop the `rust_call_arg`-needs-a-fix claim. Doc-accuracy — a maintainer reading it
would chase a non-existent blocker.

Fix (per Copilot): update the comment to state the ACTUAL remaining reason `s2_arg_ok` doesn't admit a
Record arg yet (likely just "not yet added / a follow-up slice"), or drop the `rust_call_arg` claim.
Doc-only. Owner: v-rust-backend (`backend/rust/mod.rs` + `cdz-rust-render`; commit `99a70ca02`). Routed
as a note.
