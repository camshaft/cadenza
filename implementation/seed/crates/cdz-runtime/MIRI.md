# Running cdz-runtime under Miri (UB-freedom check)

The runtime's reference counting is hand-written `unsafe`-adjacent accounting: `dup`/`drop`/reclaim,
FBIP in-place reuse (`rc == 1`), and the Perceus husk-reclaim paths. Miri is the oracle that catches
an aliasing/use-after-free/double-free mistake in that accounting that the native allocation-bench and
the corpus byte-parity gates cannot see. Any refcount-sensitive change (an FBIP move-not-clone, an
owned-slice rc transfer, a consume-with-ownership reclaim) should be re-verified under Miri.

## The one command

From the **repository root** (not this crate directory — see "Why" below):

```sh
implementation/seed/crates/cdz-runtime/miri.sh [TEST_FILTER]
```

`miri.sh` bakes the full recipe. With no argument it runs the whole crate under Miri; with a filter it
runs only the matching tests, e.g. the rc-accounting proptest:

```sh
implementation/seed/crates/cdz-runtime/miri.sh prop_bytes_matches_reference_under_random_op_sequences
```

Equivalent raw invocation (what the script runs):

```sh
MIRIFLAGS="-Zmiri-ignore-leaks -Zmiri-disable-isolation" \
RUST_MIN_STACK=67108864 \
cargo +nightly miri test \
  --manifest-path implementation/seed/crates/cdz-runtime/Cargo.toml \
  [TEST_FILTER]
```

## Why each piece is load-bearing

- **Run from the repo ROOT, never from inside this crate.** This crate's `.cargo/config.toml` carries
  an `[unstable] build-std = ["core", "alloc", "panic_abort"]` block (it makes the shipped wasm bytes
  byte-identical across host architectures by compiling the panic machinery out). Nightly Miri *honors*
  `build-std`, which makes it rebuild `core` and then collide with its own sysroot `core` —
  `error[E0152]: duplicate lang item in crate 'core'`. Cargo discovers `.cargo/config.toml` from the
  **current working directory upward**, not from the manifest's directory, so running from the repo root
  (whose `.cargo/config.toml` has no `build-std`) with `--manifest-path` pointed at this crate sidesteps
  the whole problem. A native `cargo test` from inside the crate is unaffected: without `RUSTC_BOOTSTRAP`
  cargo silently ignores the `[unstable]` block, and `cargo +nightly miri` would NOT.

- **`RUST_MIN_STACK=67108864` — a plain byte count, NOT `64M`.** The proptests recurse deeply enough to
  overflow Miri's default interpreter stack. The value must be a bare number of bytes; a suffixed form
  like `64M` is rejected by rustc (`` `RUST_MIN_STACK` should be a number of bytes ``) and aborts the run
  before any test executes. `67108864` is 64 MiB.

- **`-Zmiri-ignore-leaks`** — the runtime holds deliberate process-lifetime immortals (`EMPTY_BYTES`,
  `EMPTY_STR`, interned singletons) that are never freed by design; without this flag Miri reports them
  as leaks and fails a clean run. Leak accounting for the *reclaimable* heap is covered separately by the
  native `--features debug-counters` live-objects drift guard, not by Miri.

- **`-Zmiri-disable-isolation`** — the proptests seed their RNG from the clock/OS entropy; isolation
  blocks those syscalls.

## Gotchas when verifying by hand

- Do **not** pipe the invocation through `| tail` / `| head` without capturing the exit status — a
  recipe error (e.g. the `RUST_MIN_STACK` rejection above) prints to stderr while the pipeline exits 0,
  masking a run that never actually tested anything. `miri.sh` runs unpiped and propagates the real exit
  code.
- Nightly is required (`cargo +nightly miri`); the pinned stable toolchain has no Miri. The `nightly`
  rustup toolchain with the `miri` component installed is what the recipe uses.

## A supported `cargo xtask miri` entry point

The dev-facing one-command form (`cargo xtask miri [--filter]`) is being wired as an xtask/nix entry
point in coordination with the build-tooling owner; this script + doc are the verified recipe it wraps.
See board task_731.
