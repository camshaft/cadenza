# The native Cadenza agent harness

An agent **loop authored in Cadenza itself** — the piece that replaces the headless `claude` CLI in the
fleet's / hivemind's worker: read a message → call a model (Amazon Bedrock, direct) → dispatch tools,
with every tool dispatch authorized by a Cedar policy. It is the flagship **dogfood**: the fleet building
its own runtime in the language it compiles. Design: `implementation/design/DESIGN-agent-harness.md`.

## The two halves

- **The loop, in Cadenza** (this package, `implementation/agent-harness/`): the pure-Cadenza control
  structure. `src/loop.cdz` is the loop spine (a recursive turn loop over `Model`/`Tools` effects);
  `src/authz-loop.cdz` adds the Cedar authorization gate (perform `Cedar.authorize(action)` before every
  `Tools.dispatch`, dispatch only on allow — "no ambient authority"). The loop's external interactions
  are **effects** (`Inbox.next`, `Model.converse`, `Cedar.authorize`), so an in-program `handle` mocks
  them for tests and the SAME loop routes to real backends with no code change (a nearer handler wins).

- **The embedder, in Rust** (`implementation/seed/crates/cdz-agent`): the thin non-Cadenza host that
  RUNS the loop and answers its effects. It reuses `cdz-run`'s `run_agent_hosted` to bind each effect op
  to a host closure over the shared value-heap runtime (the loop's `String` prompt/completion/action
  cross as runtime rope handles, marshaled with the runtime's `str-get`/`str-new`):
  - `Model.converse` → `mock_converse` (tests) or `bedrock_converse` (real `aws-sdk-bedrockruntime`
    InvokeModel, behind `--features bedrock`). A failed model call is reported as a failure, never a
    silent answer (`MODEL_ERROR_PREFIX` / `is_model_error`).
  - `Cedar.authorize` → `cedar_authorizer` / `cedar_delegated_authorizer` over the real `cedar-policy`
    evaluator (`src/cedar.rs`), fail-closed; on-behalf-of = agent-policies ∩ user-delegation.
  - `Inbox.next` → a closure handing the loop the current message body.
  The only non-Cadenza surface is these closures; the agent loop itself is pure Cadenza.

## Why an embedder and not a Cadenza peer or a host op

A Cadenza `String` peer op crosses the component boundary as an opaque `u32` runtime HANDLE (the provider
imports the value-heap runtime), so a naive WASI `converse(string)->string` peer can't answer it; and a
host op can't yet RETURN a `String` (the host-result ABI is unbuilt). The embedder sidesteps both by
binding the effect to a Rust closure that reads/mints rope handles against the shared runtime. This is
the concierge-approved **bring-up** (design §7 option c); when the host-`String`-result ABI lands it can
collapse to a cleaner host binding.

## Run it

```
# Tests (mock model, no creds/network):
cargo test --manifest-path implementation/seed/crates/cdz-agent/Cargo.toml

# Test the real Bedrock backend compiles + its decode unit tests:
cargo test --features bedrock --manifest-path implementation/seed/crates/cdz-agent/Cargo.toml

# The @test suite for the Cadenza loop package:
cdz test implementation/agent-harness

# Drive a real inbox (mock model, permit-all demo policy):
cargo build --manifest-path implementation/seed/crates/cdz-agent/Cargo.toml
cargo xtask build   # the value-heap runtime store the loop needs
implementation/seed/crates/cdz-agent/target/debug/cdz-agent \
    --consumer <a-compiled-loop.wasm> --inbox <a-fleet-inbox-dir> [--policy <cedar-file>]
#   --features bedrock + --model <id> drives a real Bedrock model.
```

`cdz-agent` is a **workspace-excluded** crate (its own `[workspace]`, like `cdz-cad`) so its heavy async
aws-sdk tree never burdens the seed build/gate; a dedicated CI job builds+tests it (`.github/workflows/
checks.yml`, the `cdz-agent` job). The loop package's `@test` suite runs in the `cadenza @test suites`
CI job + `cargo xtask check`.

## Status (2026-07-16)

Charter pieces 1–3 SHIPPED + hardened: Bedrock-direct (embedder), the loop in Cadenza (loop.cdz + the
driver binary reading a real inbox and returning the model's actual completion), Cedar permissions +
on-behalf-of (authz gate + delegation with expiry/forbid coverage). Piece 4 — self-modification /
evolvable toolchains (Inc-4) — is held pending the operator (add `rcdzc` as a dep for runtime
compile-a-new-tool; Cedar-gate + optionally v-verification-prove each self-mod).

Like the compiler-ml port, this is a **stress test of the language**: gaps found here are reported
(REPORT/FIX), not worked around. This vertical's probes drove several `v-effects` + `v-peer-linking`
fixes (peer `String` argument/result-escape emit, the multi-peer fused resource envelope, the
effectful-helper-in-a-recursive-self-call specialization family).
