# DESIGN: source export — nix flake outputs → vendorable Rust source

Owner: `v-nix-projection` (subsystem `nix-flake`). First consumer: MembrainHivemind (a package
that plugs the REAL Cadenza reducer as its per-event fold), via `v-hivemind` — but nothing here
is consumer-specific; this is a generic source export.

## Problem

A downstream consumer builds in an environment with no nix (e.g. a cargo-only or corporate build
system). It needs the Cadenza reducer-runtime pieces as **vendorable Rust source** it can commit +
build with cargo. The naive options are both bad:

- **nix in the consumer's build** — the consumer has no nix; a non-starter.
- **hand-codegen / a fork per piece** — drifts from Cadenza the moment either side changes.

We want a **faithful, re-runnable PROJECTION**: a single source of truth (Cadenza's flake +
its pinned inputs) from which the vendored Rust tree is regenerated on demand, so the exported
copy can never silently drift.

## The three pieces to project (v-hivemind's request)

1. The reducer-world host imports — `run` / `blobs` / … (NOT `heap`; see below).
2. The wasmtime driving used to instantiate + drive a reducer-world component.
3. The binary-AST value codec (encode the event value, decode the outcome/requests).

Terminology correction locked in during scoping: **`heap` is NOT a reducer host import.** It
is a separate ABI — the tag-free value-heap runtime `cadenza:runtime` (crate `cdz-runtime`,
wasm32-only) that the compiler-emitted program imports by hash. It is out of scope unless the
consumer's reducer actually needs the value-heap component (a distinct wasm-component
projection).

## Drift-proof approach

The pieces are already first-party Rust crates, pinned + vendored by the flake:
`seedCargoVendor = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; }`. The
projection reads the **same pinned inputs the flake builds from**, so drift is impossible by
construction:

- **Sources** are copied verbatim from the repo crates. Build-time-GENERATED code that a projected
  crate needs (e.g. `cdz-platform/src/contracts/`, produced by the `cdzPlatformContracts` codegen and
  gitignored in-repo) is overlaid into the projected crate, exactly as the flake's own builds do — so
  the projected tree is codegen-complete and compiles standalone.
- The projected **crate SET** is *auto-discovered* from the tier's root crates' manifests (transitive
  `path = "../…"` sibling deps), so it auto-adapts if an upstream crate gains a first-party dep.
- The projected **`Cargo.lock`** is *derived by filtering* the repo's pinned root `Cargo.lock`
  to the transitive, **version-aware** dependency closure of the projected crates — no fresh
  resolution, so the emitted versions are exactly what the nix build pins. Version-aware so
  duplicate-version crates (e.g. three `syn`s in the root lock) are not over-included; the
  emitted lock stays minimal.

A small std-only tool (`cdz-source-export`, an excluded zero-dep crate) performs the copy
+ lock-filter; a nix derivation wraps it so a refresh is one command.

## Tiers (kept separate so wasmtime never leaks into the light path)

The seed workspace deliberately excludes the http/wasmtime crates so cargo feature-unification
never forces wasmtime into routine builds. The projection preserves that boundary by projecting
in **separate opt-in tiers**:

- **Tier A — codec (LIGHT, DELIVERED).** `cadenza-ast` + `cadenza-value` + `cadenza-ast-serde`.
  No wasmtime/tokio/network; `cadenza-ast`'s core is `no_std`. This is piece #3 (the value
  codec) and the foundation the other pieces cross their value boundaries through.
- **Tier B — reducer (HEAVY, DELIVERED).** `cdz-platform` (with its `host` feature = wasmtime 37
  + cranelift + tokio) + first-party deps `cadenza-ast` / `cdz-contract` / `cdz-str` + the WIT
  world (`cdz-platform/wit/world.wit`). This is pieces #1 + #2: the reducer-world host-import
  traits (`BlobStore` / `KvStore` / `ReducerGraph` — the consumer plugs its own impls) and the
  wasmtime driving (`ReducerHost` / `WasmReducer` / `WasmProgramStore` in `cdz-platform/src/
  host.rs`). Excludes `itest-alloc` (jemalloc's C build fails in minimal sandboxes) and
  `testing` (the bach simulator). Projected as its own opt-in package so wasmtime stays out of
  Tier A.

## Mechanics

- **Tool:** `cargo run -p cdz-source-export -- --repo <cadenza> --out <tree>`.
- **Refresh (drift-proof, from the flake):** `nix build .#codec-source-export && cp -r
  result <workspace>`.
- **Package:** `packages.codec-source-export` runs the tool against the flake's pinned
  sources → a self-contained cargo workspace at `$out` (`crates/<name>` + workspace
  `Cargo.toml` + filtered `Cargo.lock` + `REFRESH.md`).
- **Gate coverage:** `checks.<sys>.codec-source-export` offline-builds the projected tree
  against `seedCargoVendor` — a green proves the tree is self-contained and vendorable with the
  exact pinned versions. STANDALONE (not in `local-gate`, like the `cdz-http-*` checks) so a
  projection tool never burdens the fleet's merge gate.
- The check uses `--offline` (not `--locked`): a vendored-sources source-replacement rewrites
  the projected lock's `source` fields (the projected workspace differs from the root workspace
  the vendor was built from), so `--locked` trips benignly on a perfectly buildable tree; lock
  exactness is asserted separately by the tool's unit tests + a `--locked --offline` build
  against a real registry.

## Status

- **Tier A (codec):** DELIVERED — tool (#8858) + nix package/check (#8860). Refresh:
  `nix build .#codec-source-export`.
- **Tier B (reducer):** DELIVERED — `--tier reducer` (#8867) + nix package/check (#8868). Refresh:
  `nix build .#reducer-source-export`; build the projected tree with `--features cdz-platform/host`.
  The consumer depends on the projected `cdz-platform` host surface and wraps `WasmReducer` in its
  `ReducerRuntime::fold`, plugging its own CAS/heap/delegate impls into the host-import traits.
- **Crate-name prefix:** a `--prefix <P>` option (namespacing the exported crate names) is planned so
  a consumer can vendor the crates under a namespaced name to avoid collisions.
