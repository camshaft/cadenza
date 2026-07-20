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

## The minimal-kernel daemon (`cdz-kernel` + `cdz-agent` CLI)

Alongside the embedder, the vertical shipped the operator's **minimal-kernel** re-charter
(`implementation/design/DESIGN-agent-runtime-minimal-kernel.md`): a log-native agent runtime where the
Rust host understands NO events — a self-modifiable **Cadenza `interpret` program in the log** decides
everything, and the kernel just compiles+runs it (`cdz-kernel`, `src/kernel.rs`) over an append-only event
log (`Log` trait; `FileLog` local backend, DynamoDB L1d marshalling behind `--features aws`).

The `cdz-agent` CLI (`implementation/seed/crates/cdz-kernel/src/bin/cdz-agent.rs`) is the thin operator
surface — everything is data in the log:
- **bootstrap / inject-genesis** — create the log; seed the genesis interpret program (a later inject
  self-supersedes it, so even the first program is self-modifiable).
- **emit / run / perform / hosted** — append a trigger; drive one daemon tick that COUNTS (`run`),
  EXECUTES the scheduled ops (`perform`), or executes them via REAL host primitives recording each to the
  log (`hosted`, the K1c→host rung). `hosted --policies <cedar>` Cedar-gates each primitive.
- **emit-policy / authz-grant / authz-revoke / authz-requests** — the capability model: Cedar policy docs
  in the log attenuate each invocation (deny-by-default); operator grants (optionally wall-clock-expiring)
  widen within what the operator writes; a denied op auto-files an `authz-request` (the can't-brick hatch).
- **schedule-create / schedule-cancel / schedule-list** — one-shot + periodic timers in the log (COALESCE
  a backlog into one fire); the live daemon fires due schedules each round.
- **replay** — RE-FOLD a recorded turn from the `prim-result-*` trail with NO live effect (the §2.3
  recorded-effect determinism proof: same cognition, world non-determinism frozen).
- **fork** — branch a recorded history into a new timeline (`--upto <seq>` for a time-travel cutoff); the
  branch re-folds + extends independently (drops the parent's resume bookmark, keeps the effect trail).
- **cursor** — show the daemon's resume high-water mark + pending-trigger backlog (read-only).
- **start** — the LIVE daemon (poll → fire schedules → perform each new trigger → sleep). CRASH-RECOVERY:
  it durably records a `daemon-cursor` high-water mark, so a restart AUTO-RESUMES where it left off
  (`--from <seq>` overrides) and re-performs nothing — at-most-once.

Record → replay → fork → crash-recovery are the four faces of the log-as-source-of-truth model, all pure
folds over the event log. Gated in-tree: `cargo test -p cdz-kernel` (lib + the `cdz-agent` CLI integration
suite), plus `fmt`/`clippy` under both the default and `--features live-exec` builds.

## Status (2026-07-20)

**Embedder (charter pieces 1–3) SHIPPED + hardened:** Bedrock-direct, the loop in Cadenza (loop.cdz + the
driver binary reading a real inbox and returning the model's actual completion), Cedar permissions +
on-behalf-of (authz gate + delegation with expiry/forbid coverage).

**Minimal-kernel daemon COMPLETE + cross-feature-gated:** the 7-verb `cdz-agent` surface above +
record/replay/fork/crash-recovery, with every invariant witnessed by the gate — including the fork×replay
composition (a forked timeline is replay-faithful) and the operator-can't-forge-the-`daemon-cursor` guard.

**Held pending the operator:** piece 4 — self-modification / evolvable toolchains (Inc-4: add `rcdzc` as a
dep for runtime compile-a-new-tool; Cedar-gate + optionally v-verification-prove each self-mod) — and the
L1d DynamoDB CLIENT wiring (the many-writer ordering authority: conditional `PutItem`; needs a live table,
so the marshalling is tested in-tree but the network calls are `--features aws`, exercised manually).

Like the compiler-ml port, this is a **stress test of the language**: gaps found here are reported
(REPORT/FIX), not worked around. This vertical's probes drove several `v-effects` + `v-peer-linking`
fixes (peer `String` argument/result-escape emit, the multi-peer fused resource envelope, the
effectful-helper-in-a-recursive-self-call specialization family).
