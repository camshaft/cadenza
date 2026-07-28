# PR#722 review comments — stale compile-stack comments (compile_component no longer wraps)

Mirrored from GitHub PR review comments (Copilot), ids `3619635024`, `3619635055`.
PR: https://github.com/camshaft/cadenza/pull/722 (merged; fix still belongs on trunk)
Locations: `implementation/seed/crates/rcdzc/src/host.rs:86`, `implementation/seed/crates/rcdzc/src/compile.rs:106`.

## Comments (verbatim)

- (id 3619635024, host.rs:86) "The comment describing why `run_with_compiler_stack` can nest is now
  stale: `compile_component` no longer wraps compilation with `run_with_compiler_stack` (the wrapper
  moved to `compile_with_opt`). This makes the rationale misleading and may confuse future changes to
  stack-guard routing."
- (id 3619635055, compile.rs:106) "This comment still refers to an 'existing outer wrap' in
  `compile_component`, but `compile_component` no longer wraps compilation in
  `run_with_compiler_stack`. Updating this wording will keep the rationale accurate now that the stack
  precondition is established in `compile_with_opt`."

## Liaison verification (CONFIRMED on trunk)

- `compile.rs:448` `compile_component` body just calls `compile(&[Artifact::new(KIND_AST,...)], &[Target::Wasm])`
  — it does NOT call `run_with_compiler_stack`. The wrap now lives once at the shared sink in
  `compile_with_opt` (compile.rs ~107: `crate::host::run_with_compiler_stack(|| compile_with_opt_inner(...))`).
- `host.rs:83-86` comment still says: "`compile_with_opt` establishes the precondition at the shared
  sink, but `compile_component` and the bin ALSO wrap their `compile` call" — the `compile_component`
  half is now false.
- `compile.rs:104-105` comment still says: "the bin's and `compile_component`'s existing outer wrap
  does NOT double-spawn" — again, `compile_component` no longer wraps.

Fix: drop the `compile_component` mention from both comments (the bin may still wrap — verify; the
idempotence rationale stays valid for the bin + nested test callers). Doc-only, no behavior change.
This is v-runtime's compile-stack routing (landed trunk `97e8ac12d` = MR integrated at `3799ed90d`,
"route ALL compile entry points through the guard-sized worker stack"). Routed as a note to v-runtime.
