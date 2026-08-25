# `cdz` delegates compilation to the external `cdz-compile` CLI (a build-time feature)

Status: DESIGN (v-cdz-delegate, 2026-08-25, operator-requested). Goal: a cargo feature on the
top-level `cdz` binary that makes it **delegate compilation to the standalone `cdz-compile`
process** instead of linking the compiler (`rcdzc`) in-process — so a compiler change no longer
forces a `cdz` rebuild. Nix turns the feature **on by default** for the packaged `cdz`, so the
compiler and `cdz` cache/rebuild independently.

Operator directive (verbatim): "we just merged a change to compile a standalone cdz-compile cli. i
would like to add a feature flag to the top level cdz binary to avoid pulling in the compiler
directly and instead delegating to the external process. and in nix we should do that by default.
that way we get better caching."

## Why this pays off

`cdz` is the unified toolchain binary. Today it links **`rcdzc`** (the reference compiler) directly:
`cdz compile`, the project commands (`build`/`run`/`test`), and every span-mapped semantic query
(`type`/`type-at`/`uses`/`check`/`fix`/`def`/`scope`), the LSP server, and `doc-module` all call the
compiler in-process. So **any** compiler-internals change rotates the `cdz` derivation — and in the
nix graph a great deal keys off the packaged `cdz` (`seedCompiler`: it drives `cdz rewrite` / `cdz
convert` at eval, and `cdz-contract` shells `cdz hash`; harness-program and contract-hash
derivations chain off it). Those eval-time uses are **front-end (syntax) operations that don't need
the compiler at all**, yet they rebuild whenever the compiler changes.

`cdz-compile` (#3388) already carved the compiler out as a standalone bin with a compiler-only
dependency closure, and the corpus **build** phase already uses it. This design extends the same
split to the *packaged toolchain*: if `cdz` reaches the compiler by **spawning `cdz-compile`**
rather than linking `rcdzc`, then a compiler-only change leaves `cdz`'s own inputs untouched — it is
a cache hit, and everything keyed on it stays warm.

## The seam is unusually clean

`cdz-compile` is a thin shim over `rcdzc::cli::run` → `run_prepared` — the **exact** host-boundary
code the in-process `cdz compile` already calls. So delegating is not a reimplementation; it is
moving the same call across a process boundary:

- **Inputs.** `run`/`run_prepared` read named artifacts (`kind:name=path`), inject `--entry` /
  `--component-name` artifacts, and apply the `[Wasm]` target default. `cdz` already builds exactly
  this artifact set (parsing SOURCE files in-process with `cadenza-syntax` into `ast`/`spans`
  artifacts — see `run_compile`/`compile_source_specs`). To delegate: materialize each artifact to a
  temp file and hand `cdz-compile` a `kind:name=<tmp>` spec.
- **Output.** `run_prepared` writes each artifact to `-o` (a file, a directory, or `-` = stdout) —
  behavior we forward verbatim by passing `-o` through unchanged.
- **Diagnostics, located identically.** `run_prepared` maps a diagnostic's node id back to
  `path:line:col` **from the `spans` artifacts it is given**. Because we pass the same `spans`
  artifacts to `cdz-compile`, its stderr is byte-for-byte the located diagnostics `cdz` prints today
  — no in-process span remap needed on the compile path.
- **Exit status.** Forwarded (the `passthrough_status` / `exit_code_from_child` helpers already
  handle non-`0..=255` and signal-killed children correctly).

Net: on the compile path, delegation is *materialize artifacts → build argv → spawn → forward
stdout/stderr/exit*. A pure artifacts-in invocation (`cdz compile kind:name=x.ast …`) needs no temp
files at all — argv forwards directly.

### Semantic queries and the LSP (the invasive part)

The query surfaces (`type-at`, `def`, `scope`, `uses`, `check`, `fix`, LSP, `doc-module`) drive the
compiler's **sidecar**: they build in-memory `ast` + `spans` + a `KIND_SIDECAR` request artifact,
call `rcdzc::compile`, and read a result artifact (`KIND_TYPE_AT`, `KIND_DIAGNOSTICS`, …), then map
node ids back to source spans **in-process**. `cdz-compile`'s `run_prepared` **already handles
sidecar inputs** (the `has_sidecar` branch produces no default `wasm` target and writes the sidecar
result artifacts to `-o`). So a query delegates the same way as compile: write `ast`/`spans`/sidecar
temp files, spawn `cdz-compile -o <tmpdir>`, read the result artifact file back, and do the
node-id→span mapping in-process (the span tables never leave `cdz`). This is mechanical but touches
a lot of call sites in `main.rs` + `lsp.rs`.

## Locating `cdz-compile`

Reuse the established sibling-passthrough convention (`cdz smith` / `cdz cad`):
`locate_sibling_bin("cdz-compile")` → `current_exe().parent()/cdz-compile[.exe]` if it exists, else
fall back to `cdz-compile` on `$PATH`. Add one override for the packaged/nix case: honor
`$CDZ_COMPILE_BIN` (an explicit absolute path) first, so the flake can inject the exact
content-addressed `cdz-compile` from the `cdz-compile` derivation's `bin/` — no `$PATH`/CWD
ambiguity, and the two derivations stay independently cached. Resolution order:
`$CDZ_COMPILE_BIN` → sibling → `$PATH`. A NotFound spawn gets an actionable error (`build it with
cargo build -p rcdzc --bin cdz-compile`, mirroring `passthrough_status`).

## The scope decision (routed to the operator as an `ask`)

The caching win is realized **only if `cdz` links zero `rcdzc`** under the feature — which means
*every* rcdzc-backed surface must delegate, not just `cdz compile`. Three shapes:

- **(A) Full delegation** — gate every rcdzc surface (compile + all queries + LSP + doc-module +
  project build/run/test) behind the feature; make `rcdzc` an **optional** dependency
  (`dep:rcdzc`). Only option that actually drops `rcdzc` from `cdz`'s closure → the stated caching
  goal. Larger, multi-slice rework of `main.rs`/`lsp.rs`; queries/LSP pay one subprocess spawn per
  request **when the feature is on** — but the feature is nix-only; dev/interactive `cdz` keeps the
  in-process (feature-off) path and stays fast.
- **(B) Compile-only delegation** — only the compile paths delegate; queries/LSP stay in-process.
  Small and clean, but `rcdzc` still links, so **no caching benefit** — mostly symbolic.
- **(C) A separate slim binary** — a compile+syntax-only tool that delegates, leaving full `cdz`
  untouched; nix packages the slim one where caching matters.

**Recommendation: (A).** It is the only option that delivers the caching goal, the subprocess cost
bites only the nix build, and dev/interactive `cdz` is unchanged. (C) duplicates the CLI surface;
(B) does not move the needle. Awaiting the operator's confirmation before the invasive
`rcdzc`-optional rework.

## Nix (v-nix's single-writer flake — announce-before-touch)

The flake is v-nix's territory; the operator said keep it off v-nix's plate, so **I** drive it, but
I announce-before-touch and hand v-nix the exact hunk (coordination note already sent). The change
is minimal:

1. `seedCompiler` builds `-p cdz -p cdz-run --no-default-features`; add `-p rcdzc` (to co-produce
   `cdz-compile` in the same `bin/`) and the `delegate-compile` feature to the `cdz` build.
2. Ensure `cdz-compile` lands in the same output `bin/` as `cdz` so `locate_sibling_bin` finds it
   (or set `$CDZ_COMPILE_BIN` in the `cdz` wrapper to the `cdz-compile` derivation's path — the
   content-address route, which keeps the two derivations independently cached and is the cleaner
   fit for "better caching").

## Incremental slice plan

0. **This design doc.** (done)
1. **Delegation core + compile path** — `delegate.rs`: `locate_cdz_compile()`
   (`$CDZ_COMPILE_BIN`→sibling→`$PATH`), a `delegate_compile(artifacts, targets, out, opt_level)`
   that materializes temp files, builds argv, spawns, forwards exit/stderr; add the
   `delegate-compile` cargo feature; route `run_compile`/`compile_source_specs` through it when the
   feature is on. Gate: a feature-gated integration test that builds a program via the delegation
   path and asserts byte-identical output to the in-process path.
2. **Query delegation** — route the sidecar-driven queries (`type-at`/`def`/`scope`/`uses`/`check`/
   `fix`/`doc-module`) through `cdz-compile` under the feature; keep node-id→span mapping in-process.
3. **LSP delegation** — same for `lsp.rs`.
4. **Flip `rcdzc` optional** — `dep:rcdzc` behind the feature's inverse; confirm a feature-on build
   has no `rcdzc` in its closure (`cargo tree`). This is the slice that realizes the caching win.
5. **Nix default-on** — the flake hunk, handed to / co-landed with v-nix.

(Slices 2–4 are gated by the operator's scope answer — under (B) only slice 1 lands; under (C) the
work moves to a new bin.)

## Invariants to pin in the gate

- Feature-off `cdz compile` output == feature-on `cdz compile` output == `cdz-compile` output, for
  single-file, multi-file package (`--entry`), and imposed-world (`--component-name`) cases.
- Located diagnostics (`path:line:col`) are byte-identical across the boundary (guaranteed by
  passing `spans` artifacts through, but pinned by a reject-case test).
- A feature-on build's dependency closure excludes `rcdzc` (slice 4 — a `cargo tree` assertion in
  `cargo xtask check`).
