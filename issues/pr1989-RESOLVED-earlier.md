# PR #1989 review — flake.nix (v-nix) — MERGED — nix-hygiene [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1989 (full-CI-in-nix increment 2 — cargo test as a nix check).
Copilot (id 3710822631) flags `seedTestSrc` includes the whole `./spec` tree though only `spec/semantics`
is needed.

## `seedTestSrc` includes all of `./spec` but the tests only resolve `spec/semantics` → over-wide input, spurious cache invalidation on non-semantics spec edits (Copilot, flake.nix:147) — nix-hygiene [VERIFIED, LOW]
> `seedTestSrc` includes the entire `./spec` tree, but the rationale above only mentions tests needing
> `spec/semantics` … Including all of `spec/` will unnecessarily widen inputs and cause cache
> invalidation/rebuilds when non-semantics spec files change.

VERIFIED. `seedTestSrc` (flake.nix:137) unions `./spec` (whole tree), while its OWN preceding comment
states the only runtime need is `spec/semantics` ("cadenza-syntax's corpus_roundtrip tests resolve
`$CARGO_MANIFEST_DIR/../../../../spec/semantics`"). So a change under `spec/design`, `spec/capabilities`,
etc. busts the test-check derivation's input hash and forces a rebuild, even though the tests never read
those. Copilot's fix (narrow to `./spec/semantics`) matches the stated rationale.

VERIFY-CAVEAT I checked (so the narrow doesn't break something): the seed workspace DOES also reference
`spec/capabilities` and `spec/contracts` — BUT only in `//=` DUVET CITATION comments (e.g. eval.rs:488
`//= spec/capabilities/core-semantics.md#…`), which are compile-time annotations baked into the .rs source
(already in `./implementation/seed/crates`) and checked by the duvet tool, NOT files the test derivation
reads at runtime. So the tests genuinely only resolve `spec/semantics`; narrowing `./spec` →
`./spec/semantics` is safe for THIS derivation. LOW/nix-hygiene. Fix: replace `./spec` with
`./spec/semantics` in the `seedTestSrc` fileset. (Flag to v-nix: confirm no other nix check reuses
`seedTestSrc` expecting the broader tree, and that duvet's own check — if it runs in nix — has its own spec
input.) v-nix owns flake.nix.
