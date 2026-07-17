# L1 implementation plan — the fold owner over a log (agent-runtime ladder rung 1)

**Owner:** v-agent-harness (implementation). **Charter:** `DESIGN-agent-runtime-vision.md` (the elevated
vision, operator-locked 2026-07-16) is now this vertical's north star; the shipped Inc 0–3 (Bedrock
embedder, Cedar authorizer, Cadenza loop package) is its **L0**. This doc plans **L1** — the smallest rung
that demonstrates the microkernel thesis "minimal core = tail → fold → execute effect-requests."

> Status: PLAN (this doc). No kernel code yet. The assign (concierge 2026-07-16) says build L1 on the
> default that v-agent-harness owns it; the assign-vs-mint-a-new-vertical org call is with the operator, so
> this plan is written to be hand-off-safe (a new `area=agent-runtime` vertical could pick it up verbatim).

## L1 goal (vision §15)

A single-threaded **Rust fold owner** that: tails an ordered **log** → **folds** it with a **Cadenza
program** → drives **one agent loop end-to-end**, reusing the shipped Bedrock embedder for the model call.
Proves the microkernel shape + **recorded-effect determinism** (vision §2.3) against a real log.

## What L1 REUSES (already shipped — this is why L1 is small)

- **Recorded-effect determinism** — `cdz_run::RunOpts::host_responses` + `bind_host_imports` already
  replay recorded host results in call order. This IS the §2.3 mechanism: the fold emits an effect-request,
  the kernel performs it live and appends the response event; on replay the recorded response is reused.
- **The N-op host driver** — `cdz_run::run_agent_hosted` binds N host ops (inbox/model/cedar) to closures
  over the shared value-heap runtime. The fold owner drives the Cadenza fold program through this.
- **The Bedrock embedder** — `cdz_agent::bedrock_converse` (+ mock) is the model effect's live handler.
- **The Cadenza loop** — the L0 loop modules are the "agent loop" L1 drives.

## The single hard constraint L1 lives inside (vision §2.3 + my constraint brief)

Determinism-by-recording is the whole game: **every non-deterministic touch (model call, clock, build) is
an effect-request the fold emits; its result is appended as an immutable event.** Live = perform + append;
replay = reuse the recorded event. The fold over `(request-event, response-event)` is then PURE. L1 must
demonstrate exactly this: run live once (recording model responses into the log), then RE-FOLD the same
log slice and get the identical outcome with no model call (proving replay determinism).

## L1 decomposed into gated sub-rungs (one MR each, sequential)

- **L1a — the log abstraction + an in-memory/file log (NOT DynamoDB yet).** A minimal `Log` trait
  (`append(event) -> seq`, `tail(from_seq) -> events`) with a deterministic file-backed impl. Event =
  a tagged record (kind + payload bytes + seq). Rationale: prove the fold/replay shape against a LOCAL log
  first (no AWS creds/network in CI), exactly as the embedder shipped mock-first. DynamoDB is L1d.
- **L1b — the fold owner loop (single-threaded).** A Rust owner that `tail`s the log, and for each
  agent-runnable state folds a **Cadenza fold program** (via `run_agent_hosted`) that decides the next
  action. The model effect is bound to the embedder; the owner appends the model response as an event.
  Deliverable: drive ONE agent loop turn end-to-end from a log, appending request+response events.
- **L1c — replay determinism gate.** Run L1b live (records responses), then RE-FOLD the same log with the
  model handler REPLACED by "reuse recorded response events" — assert identical outcome, zero live model
  calls. This is the load-bearing L1 proof (the §2.3 thesis, gated).
- **L1d — swap the file log for DynamoDB.** Behind an `aws`/`dynamodb` feature (like `bedrock`): the
  conditional-write `seq` assignment (ordering authority, §2.1) + DynamoDB-Streams/poll tail. CI keeps the
  file log; the DynamoDB path is feature-gated + unit-tested on the marshalling, network only in a manual
  run — mirroring how `bedrock` is structured.

## Crate shape

A new **workspace-excluded** crate `implementation/seed/crates/cdz-kernel` (the fold owner), same isolation
pattern as `cdz-agent`/`cdz-cad`: its own `[workspace]`, root `Cargo.toml` exclude, dedicated CI job. It
path-deps `cdz-run` + `cdz-agent` (reusing the embedder + host driver). The `dynamodb` feature (L1d) pulls
the aws-sdk the way `cdz-agent`'s `bedrock` feature does, so the default build/gate carries no AWS tree.

## Gate (per rung)

`cargo test` in `cdz-kernel` (file-log + mock-model, no network/creds — CI-safe), clippy `-D` + fmt both
feature sets; the replay-determinism test (L1c) is the invariant that pins the thesis. No corpus/runtime
change (a new excluded crate, like cdz-agent).

## Open (don't block L1; from vision §16 leaf-level)

Snapshot cadence for owner-failover re-fold (L1 re-folds from seq 0 — fine at L1 scale); the subscription
predicate language (L3); S3-vs-query-DB body tier (later). The self-mod ceiling is an L6 question.

## First action next tick

Start **L1a**: create the `cdz-kernel` crate skeleton + the `Log` trait + file-backed impl + its first
test (append/tail round-trip, deterministic seq). Gate + MR. Then L1b.
