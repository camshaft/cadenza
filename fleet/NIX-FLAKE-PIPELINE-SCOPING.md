# Scoping: move the build/test pipeline to a Nix flake

Status: FEASIBILITY READ (v-fleet-tooling, 2026-08-02). Owner: `v-fleet-tooling` (owns gate/CI/build
pipeline). Trigger: operator idea relayed by concierge (note seq 32/33) — a Nix flake that builds +
wires *everything*, with incremental test-skipping, no committed-wasm, and a shared cache.

This is a FEASIBILITY + INCREMENTAL-PATH read, not a commitment. The CI-gated parallel-lanes rewire
(`fleet/CI-GATED-LANES-DESIGN.md`) remains priority #1; this is evaluated alongside, not instead.

## The operator's pitch (verbatim intent)

> "move our whole build and testing pipeline to use nix … a lot of one off things be done in the GHA
> yaml or in xtask or just some random integration test … a nix flake that built and wired up
> everything, including being able to run tests … skipping tests that haven't changed … fix the whole
> thing about committing wasm files to the repo … share a nix cache as well."

Three concrete wins wanted: (1) incremental test-skip (content-addressed derivations), (2) stop
committing built wasm, (3) shared cache (cachix).

## Ground-truth corrections (verified against the tree, 2026-08-02)

The note's stated evidence needs two corrections before scoping — they change the starting point but
not the merit of the idea:

1. **"cachix already wired … see `.github/workflows/k-framework.yml:26`" is STALE.** There is NO
   `k-framework.yml`, NO `flake.nix`/`flake.lock`, and NO `cachix` reference in the current tree's
   **CI / Nix config** — `git ls-files` + grep over `.github/**` and any flake files = 0 hits (this
   doc's own prose aside). They DID exist historically — the K-framework reference
   implementation (PRs #141–#144: `camshaft` + `k-framework` cachix caches added to a flake and CI) —
   but that whole `reference/` subsystem, its flake, and its cachix wiring were **removed**. So we are
   NOT extending a live flake; we would be **re-introducing** one from scratch. The upside: there's a
   known-good prior-art commit to crib from (`7815edccb`, `0d625573a`) for the cachix + flake shape,
   and the `camshaft` cachix cache name/keys may still be provisioned on cachix.org (needs a check —
   raised as an ask below). Net effort is HIGHER than "extend what's wired" but the pattern is proven.

2. **The committed-wasm problem is SMALL today, but real and growing.** Exactly ONE committed built
   `.wasm` in the tree: `implementation/seed/crates/cdz-kernel/tests/fixtures/reducer_guest.component.wasm`
   (22 KB, `include_bytes!` into `component_reducer_e2e.rs`; its source `reducer-guest/` crate is
   committed and `.gitignore`s its own `/target`). The Cedar fixture the concierge flagged (the ~3.3 MB
   decision relayed to v-agent-harness-host) ships today as SOURCE (under
   `implementation/seed/crates/cdz-agent-host/tests/fixtures/cedar-policy-guest/{Cargo.toml,src,wit}`),
   built in the `cdz-agent-host` CI job — NOT as a committed binary yet. So the operator is right that
   this is the SYMPTOM to solve generally: the pattern "commit a built guest wasm because building it in
   every consumer is expensive" will recur (kernel reducer today, Cedar authorizer next, more guests
   later). A flake derivation that builds each guest once and caches it is the clean general fix — and
   it makes the Cedar-3.3 MB commit-vs-CI-build decision MOOT (flag to v-agent-harness-host).

## What "everything" is today (the sprawl to consolidate)

The pipeline logic is spread across three places — this IS the "one-off things" the operator sees:

- **`.github/workflows/checks.yml`** — 15 parallel jobs: `fmt`, `clippy`, `test` (ubuntu+macos),
  `slack-bridge`, `codegen`, `wasm-runtime`, `gate`, `cad-tests`, `cdz-cad`, `cdz-kernel`,
  `cdz-agent-host`, `rcdzc-wasm`, `roundtrip`, `bench`, `guide-examples`. Each hand-wires its own
  toolchain/target/build steps (this is exactly what the CI-lanes design leans on for parallelism).
- **`xtask`** — `build` (build value-heap runtime component + content-address into
  `target/cadenza-store`), `gate` (corpus fail-set diff), `check` (fmt+clippy+codegen), plus the fleet
  orchestration and the `@test`/`@run` runners. The store build + content-addressing is the crown-jewel
  one-off: it's bespoke content-addressed derivation logic that **Nix already does natively**.
- **Random integration tests** — `include_bytes!` of a committed wasm, per-crate fixture guests built
  ad hoc.

## Feasibility read: YES, incrementally — but it's a MULTI-WEEK arc, not a slice

Verdict: **feasible and genuinely aligned** with two live problems (committed-wasm, one-off sprawl),
BUT it is a large infra migration that must be staged so it never dark-lands or breaks the gate the
whole fleet depends on. It should NOT block or interleave-risk the CI-lanes rewire. Recommended shape:
a flake that **wraps** the existing xtask/CI logic first (parity, zero behavior change), then
absorbs one-offs incrementally, retiring hand-wired steps only after the flake proves parity.

### Incremental path (each step independently landable + gated, flake NEVER the sole gate until proven)

- **N0 — spike + prior-art salvage (this scoping's follow-on).** Resurrect the removed `flake.nix`
  skeleton from `7815edccb`/`0d625573a` as a REFERENCE (don't re-add K-framework). Confirm the
  `camshaft` cachix cache still exists + we have push credentials (ASK below). Deliverable: a
  `flake.nix` that provides a `devShell` with the pinned Rust toolchain + wasm targets + wasm-tools,
  reproducing today's CI toolchain. NO job migrated yet. Low risk (additive file).
- **N1 — flake builds the value-heap store as a derivation.** The xtask `build` step (content-address
  the runtime component into `target/cadenza-store`) is the single highest-leverage target: it IS a
  content-addressed derivation, which is Nix's native model. Wrap it as a flake output; verify the
  derivation's hash equals today's `REQUIRED_RUNTIME_HASH`. Keep `xtask build` as the source of truth;
  the flake CALLS it. Win: the store becomes cacheable + shareable via cachix (today every agent
  rebuilds it after every sync — a real, repeated cost).
- **N2 — flake builds the guest wasms as derivations → stop committing them.** Move
  `reducer_guest.component.wasm` (and pre-empt the Cedar guest) to flake-built derivations; tests
  `include_bytes!` from a Nix-provided path (or an env var the test reads). Deletes the committed
  binary; cachix serves it. This is the operator's "fix committing wasm" win, delivered concretely.
- **N3 — flake runs the test suites; wire incremental skip.** Express each `checks.yml` job as a flake
  `check`/`app`. Nix's derivation-input hashing gives "skip tests whose inputs haven't changed" for
  free at the derivation granularity (a crate whose closure is unchanged → cache hit → skipped). This
  is the biggest structural change — do it LAST, per-job, running the flake check IN PARALLEL with the
  existing job (belt-and-suspenders) until each proves parity, then retire the hand-wired job.
- **N4 — retire the one-offs.** Once N1–N3 have parity in CI for a sustained period, delete the
  now-redundant hand-wired GHA steps and fold the bespoke xtask build/store logic behind the flake.
  This composes with the CI-lanes per-lane-check-subset idea (design finding 8): a flake `check` per
  lane is the natural home for "docs lane runs a lighter subset."

### Interaction with the CI-gated lanes rewire (do NOT let them collide)

- They are ORTHOGONAL and even SYNERGISTIC: lanes decide WHICH candidate PRs run WHICH check subset in
  parallel; the flake decides HOW each check builds + whether it can be skipped/cached. N3/N4's
  per-lane flake `check`s are the concrete implementation of the design's "each lane declares its
  required check set" (finding 8) — the flake is where a lighter docs-lane subset would live.
- SEQUENCING: finish the CI-lanes executor (I4/I5) FIRST. It's priority #1, nearly drained, and it's
  the load-bearing integration change. Starting a flake migration mid-cutover would put two large
  pipeline changes in flight at once — exactly the risk the single-writer trunk model exists to avoid.
  N0 (the additive spike) can proceed in parallel since it changes nothing live; N1+ waits for lanes.

## Risks / unknowns (call out before committing)

- **CI runner Nix availability + cold-cache latency.** GitHub runners need `install-nix-action`; a
  cold cachix miss can be SLOWER than today's cargo build until the cache warms. The pilot already
  found runner-concurrency is the bottleneck (design finding 8) — a flake that lets cachix serve
  prebuilt store/guest derivations could REDUCE job time, but only once the cache is warm. Measure N1
  before promising a speed win.
- **`REQUIRED_RUNTIME_HASH` coupling.** The store hash is frozen and gate-critical. N1 must prove the
  Nix derivation reproduces the EXACT recorded hash (aarch64) or the whole gate goes red. This is the
  single most fragile step — treat it as a hard parity gate.
- **Determinism of wasm builds.** Guest wasm builds must be bit-reproducible for the derivation hash to
  be stable across machines (Nix assumes this). wasm-tools/rustc output is usually deterministic but
  needs verifying (N2 acceptance criterion).
- **Cross-arch store.** The store hash is aarch64-recorded (CI store-provisioned job). A flake must
  pin the same arch for the hash-bearing derivation or provide per-arch outputs.
- **Two-large-changes-in-flight.** Mitigated by the sequencing above (N0 parallel, N1+ after lanes).

## Recommendation

1. **Proceed with N0 now** (additive spike, zero live risk) to de-risk the "does cachix still exist +
   does the store build reproduce as a derivation" unknowns — the two facts that decide whether the
   whole idea is cheap or expensive.
2. **Hold N1–N4 behind the CI-lanes I4/I5 cutover** (priority #1, nearly drained).
3. **Raise the two operator decisions below via `ask`** (cachix account/creds + prioritization),
   because they gate real effort and only the operator can answer them.

## Asks for the operator (raised via concierge)

- **Cachix account + creds.** Does the `camshaft` cachix cache (from the removed flake, `0d625573a`)
  still exist and do we have a push auth token in CI secrets? If not, provisioning it is a prerequisite
  and a human step. (The "already wired" premise is stale — see correction 1.)
- **Prioritization.** Confirm this is N0-spike-now / N1+-after-CI-lanes-cutover, and NOT a
  drop-everything migration. The pitch reads as "explore," and the CI-lanes rewire is the standing
  priority #1 — confirming we don't invert that.

See `fleet/CI-GATED-LANES-DESIGN.md` (the priority-#1 rewire this composes with).
