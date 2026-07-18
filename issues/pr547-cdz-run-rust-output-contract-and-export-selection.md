# pr547 — `cdz run-rust` output-contract + export-selection robustness (4 Copilot comments)

Mirrored from GitHub PR #547 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/547 (publish batch, `cdz run-rust` new subcommand)
File: `implementation/seed/crates/cdz/src/main.rs` — all four in the new `run-rust` subcommand.

The subcommand's documented contract (per its own tests): emit a SINGLE verdict line on stdout
and exit 0 for ANY run outcome; the ONLY non-zero exit is reserved for input read failures.
Three of these four comments are the same violation of that contract on different failure paths;
the fourth is an export-selection ambiguity. A fix likely touches them together.

## Comment 1 — id 3607027381 (main.rs:743) — arbitrary export selection
> The default export selection for `run-rust` is derived by splitting on the first `"pub fn "`
> in the emitted Rust module. If the module contains multiple `pub fn` items (e.g. multiple
> exports), this will pick an arbitrary function and can run the wrong export. Consider
> enumerating all `pub fn` names: if exactly one exists, use it; if multiple exist, require
> `--call` (emit an `error ...` verdict) rather than guessing.

## Comment 2 — id 3607027400 (main.rs:724) — current_exe/emit harness failure breaks contract
> `cdz run-rust` is documented (and tested) as emitting a single verdict line on stdout and
> exiting 0 for any run outcome, with the only non-zero exit reserved for input read failures.
> Right now `current_exe`/emit harness failures print to stderr and return `ExitCode::FAILURE`,
> which violates that contract and can break the fuzzer/oracle harness that expects a verdict line.

## Comment 3 — id 3607027409 (main.rs:784) — rustc/driver harness failure breaks contract
> On a rustc/driver harness failure, `run-rust` currently prints an error to stderr and exits
> non-zero. This contradicts the subcommand contract (one verdict line on stdout; exit 0 for any
> run outcome) and makes failures indistinguishable from shell/harness crashes. Consider mapping
> these harness failures to an `error <msg>` verdict on stdout and exiting 0.

## Comment 4 — id 3607027420 (main.rs:905) — non-deterministic trap message
> The `trap` verdict currently uses a stderr line that often includes the temporary source file
> path/line (e.g. `panicked at /tmp/cdz-run-rust-.../prog.rs:...`), making the trap reason
> non-deterministic across runs and potentially omitting the actual panic payload message
> (depending on Rust's panic format). Extracting the panic message (next line in newer Rust, or
> the quoted payload in older formats) will make `trap <msg>` stable and more useful for
> differential comparisons.

## Triage
All four are real, substantive robustness concerns about the just-landed `cdz run-rust`
differential-orchestration subcommand — not nits. #2/#3 are the same stdout-verdict/exit-0
contract violation on two failure paths; #4 is a determinism issue that matters specifically for
the differential/oracle use case this subcommand exists for; #1 is a silent-wrong-export hazard.
Owner = v-cdz-tooling (owns the `cdz` CLI + this subcommand).
