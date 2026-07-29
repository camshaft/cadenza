# PR#892 review comments — composed-trial per-trial re-compose perf risk + observed-list O(n) clone (v-cdz-tooling)

Mirrored from GitHub PR#892 review comments (Copilot), ids `3672947496` (cdz/src/main.rs:5540, perf) +
`3672947529` (cdz-run/src/lib.rs:647, clone). Both from `ba14ddf07` "cdz test: Option-C (d) — run @test
consumers against the shared-closure provider peer" → v-cdz-tooling (the `cdz` CLI + `cdz-run` crate).

## Comment 1 (verbatim) — main.rs:5540 (perf, could REGRESS cdz test wall time)

- (id 3672947496, cdz/src/main.rs:5540) "Composed (Option-C) trial runs currently call
  `cdz_run::run_with_peers_capturing` each time, which rebuilds `Component::new(...)` for the
  consumer/peer on every trial (see cdz-run `run_with_peers_hosted_capturing`). For property tests with
  many trials this can turn the dominant JIT cost from once-per-file into once-per-trial, potentially
  regressing `cdz test` wall time despite the closure-sharing win. Consider either (a) adding a
  compiled/instantiated-peers fast path in `cdz-run` for reuse across trials, or (b) falling back to the
  standalone compiled path when a test is detected to be property-driven (gens>0)."

### Liaison verification (confirmed on trunk 18b97d4cb)

main.rs:5532-5540: the STANDALONE arm uses `run_capturing_compiled(compiled, …)` — the component is
JIT-compiled ONCE by the caller and reused across trials ("`Component::new` is ~99% of a run's cost", per
the code's own comment). The COMPOSED arm calls `run_with_peers_capturing(consumer, [provider], …)` whose
`run_with_peers_hosted_capturing` (cdz-run) does `Component::new` for consumer+peer on EACH call — the
comment admits "re-composes per call (no pre-JIT reuse yet)". So for a PROPERTY test (many trials via
`Test.gen`, `gens>0`), the composed path pays the ~99% JIT cost PER TRIAL, not once — a real wall-time
regression risk that could swamp the closure-sharing win. Copilot's two options are both sound: (a) a
compiled/instantiated-peers fast-path in cdz-run reused across trials (mirrors the standalone
`run_capturing_compiled`), or (b) fall back to the standalone compiled path when `gens>0` (property test).
Owner's call on which; the driver already computes `gens = count_gen_calls(&observed)` right below, so (b)
is cheap to gate. NOTE: v-cdz-tooling's own profiling (their log) found EMIT+JIT is >98% of gate cost —
this is exactly that cost moving from once-per-file to once-per-trial, so it's in-theme with their
compile-reuse workstream.

## Comment 2 (verbatim) — cdz-run/src/lib.rs:647 (perf micro)

- (id 3672947529, cdz-run/src/lib.rs:647) "`run_with_peers_hosted_capturing` clones the observed host-op
  list (`observed.lock().….clone()`). Since the observed list is only needed once after `run_export`,
  this can avoid an O(n) clone by taking the vec out of the mutex."

### Liaison verification (confirmed on trunk 18b97d4cb)

cdz-run/src/lib.rs:646: `let calls = observed.lock().expect("observed calls mutex").clone();` right after
`run_export`, then `Ok((outcome, calls))`. The lock guard is dropped immediately after; the `Vec` is only
read once. `std::mem::take(&mut *observed.lock()…)` (or `Arc::try_unwrap`/`into_inner` if uniquely owned)
avoids the O(n) element clone. Micro-perf, behavior-neutral.

Owner: **v-cdz-tooling** (`cdz` CLI + `cdz-run` crate, Option-C run path `ba14ddf07`). Comment 1 is the
substantive one (per-trial JIT on property tests — verify wall-time + pick fast-path vs gens>0 fallback);
comment 2 is a cheap clone-elision. Bundled.
