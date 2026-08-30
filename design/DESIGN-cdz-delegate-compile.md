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

### Sidecar request encoding — UNIFIED onto the binary AST (operator ruling, 2026-08-26)

The `KIND_SIDECAR` **request** was historically a bespoke tag+LEB128 format (a small RPC command
vocabulary: `Emit(target)` + `Query` selectors). The operator ruled it must instead be a **binary-AST
value** ("The sidecar absolutely needs to use the binary AST. I didn't realize it wasn't!"). So the
request is encoded as a small `cdzast` tree — e.g. `(query type-at 42)` — with the SAME binary-AST
codec every other artifact already uses (`KIND_AST` is `codec::encode` of the program arena; the query
RESULT artifacts are plain UTF-8 text; only this request blob was bespoke). No second wire format, no
tag table.

This **replaces** the earlier plan to extract the bespoke wire types into a shared `cdz-sidecar-wire`
crate (PR #3422, now closed): that would have been a third copy of the contract. Instead:

- **`rcdzc::sidecar`** keeps `Request`/`Query` as the in-memory types; only `encode`/`decode` change
  (tag-bytes → a `cdzast` tree built/parsed with rcdzc's OWN `crate::ast` + `crate::codec` — the
  COPIES it already carries, preserving *copy-don't-depend*, NOT a `cadenza-syntax` dep). Owned by
  **v-inference** (the active sidecar owner; concierge-routed).
- **`cdz`** (the delegating driver) builds the same request tree via its already-linked
  `cadenza-syntax` codec (byte-identical copy) — so under `!standalone` it needs **no** `rcdzc` and
  **no** bespoke crate. This is the delegation-encode side, owned here.

The only shared thing is the request-AST **schema** (the tree shape per `Request`/`Query` variant) — a
data contract, agreed between the two sides, not a code dependency. rcdzc's existing
`decode(encode(rs)) == rs` round-trip tests remain the byte-guard.

## Locating `cdz-compile`

Reuse the established sibling-passthrough convention (`cdz smith` / `cdz cad`):
`locate_sibling_bin("cdz-compile")` → `current_exe().parent()/cdz-compile[.exe]` if it exists, else
fall back to `cdz-compile` on `$PATH`. Add one override for the packaged/nix case: honor
`$CDZ_COMPILE_BIN` (an explicit absolute path) first, so the flake can inject the exact
content-addressed `cdz-compile` from the `cdz-compile` derivation's `bin/` — no `$PATH`/CWD
ambiguity, and the two derivations stay independently cached. Resolution order:
`$CDZ_COMPILE_BIN` → sibling → `$PATH`. A NotFound spawn gets an actionable error (`build it with
cargo build -p rcdzc-cli --bin cdz-compile`, mirroring `passthrough_status`).

## The scope decision — RESOLVED by the operator (2026-08-25)

The caching win is realized **only if `cdz` links zero `rcdzc`** in the delegating build — so *every*
rcdzc-backed surface must delegate, not just `cdz compile`. The operator ruled (verbatim): "we should
have a **standalone** feature that pulls everything in and bundles it rather than delegates. we can
have it **enabled by default**. but the **nix build should delegate** since it is a lot better for
caching." This is option (A) full-delegation *capability*, but framed as a **feature you turn OFF**:

- **`standalone` (ON by default)** — bundle the compiler in-process: `cdz` links `rcdzc` and does
  everything in this process (today's behavior). A plain `cargo build` / dev / interactive `cdz` is
  self-contained and fast, no external process.
- **`--no-default-features` (the NIX packaging)** — `standalone` OFF: every rcdzc surface DELEGATES
  to `cdz-compile`, and `rcdzc` is dropped from the closure. A compiler change need not rebuild `cdz`.
  The per-request subprocess cost bites only this build, which is fine for a packaged/CLI tool.

So delegation lives behind `#[cfg(not(feature = "standalone"))]`, and the final slice makes
`rcdzc = { optional = true }` with `standalone = ["dep:rcdzc"]` — a `!standalone` build then links no
`rcdzc` at all.

## Nix (v-nix's single-writer flake — v-nix OWNS + lands the hunk)

v-nix owns the flake and confirmed they'll land the hunk from a one-line proposal I hand them (no
second writer). v-nix also already builds a **separate `cdzCompile` derivation** (`rcdzc --bin
cdz-compile`) for their corpus-caching pipeline. **Decision (agreed): REUSE that derivation via
`$CDZ_COMPILE_BIN`** rather than adding `rcdzc` to `seedCompiler`'s `cargoExtraArgs` (which would
rebuild `cdz-compile` *inside* `cdz`'s closure and re-couple them — the opposite of the caching goal).
The hunk wraps the packaged `cdz` (a `!standalone` build) with
`CDZ_COMPILE_BIN=${cdzCompile}/bin/cdz-compile` → two independent content-addressed derivations.

**⚠ Landing dependency (cross-writer).** The flake's `seedCompiler` ALREADY builds `cdz`
`--no-default-features`, and `seedCompiler`'s `cdz` is used to COMPILE in these derivations:
`buildCadenzaProject` (`cdz build .`, `nativeBuildInputs = [ seedCompiler ]`) and the harness guest
build (`cdz compile … --target wasm`). Once the polarity inversion lands, those `cdz` invocations
become `!standalone` → delegate → **need `cdz-compile` reachable** (`$CDZ_COMPILE_BIN` or on the
derivation's PATH). So v-nix must wire `cdz-compile` into those derivations **before or with** the
code landing, else the nix build breaks. Also: the `delegate.rs` unit tests only compile under
`!standalone`, and `mkCrateTestCrane { crate = "cdz"; }` runs default features — so a
`--no-default-features` cdz test job is needed to keep them gated (request to v-nix).

## Incremental slice plan

0. **Design doc** — merged (#3390).
1. **Delegation core + `cdz compile`/`cdz build` path** — `delegate.rs`
   (`locate`=`$CDZ_COMPILE_BIN`→sibling→`$PATH`; `delegate_args`; `delegate_from_artifacts`), wired via
   `dispatch_compile_*`. Landed as PR #3397 (first under an opt-in `delegate-compile` flag, then
   **inverted to the `standalone` polarity** per the operator's ruling). `cdz run`/`test`'s in-memory
   compile-to-bytes path untouched.
2. **Sidecar request → binary AST** (PREREQUISITE for query delegation, per the operator ruling above).
   `rcdzc::sidecar`'s `encode`/`decode` change from tag-bytes to a `cdzast` request tree (rcdzc's own
   `crate::ast`/`crate::codec`); **v-inference** drives this rcdzc-internal redesign + defines the
   request-AST **schema**. Behavior-preserving (round-trip tests stay green). The abandoned
   `cdz-sidecar-wire` extraction (#3397-era slice "2a", PR #3422) is CLOSED — superseded by this.
3. **Query delegation** — route the sidecar-driven queries (`type-at`/`def`/`scope`/`uses`/`check`/
   `fix`/`doc-module`) through `cdz-compile` under `!standalone`: build the request tree via
   `cadenza-syntax` (per the agreed schema), write `ast`/`spans`/sidecar temp files, spawn
   `cdz-compile -o <tmpdir>`, read the result artifact, map node-ids→spans in-process. The
   `!standalone` arms reference **no** `rcdzc` types (so slice 5's flip is clean). Gated on slice 2.
4. **LSP delegation** — same for `lsp.rs`.
5. **`cdz run`/`test` in-memory compile** — delegate `compile_project_component_bytes*` (read the
   emitted wasm from a temp `-o`), the last in-process rcdzc caller.
6. **Flip `rcdzc` optional** — `rcdzc = { optional = true }`, `standalone = ["dep:rcdzc"]`; assert a
   `--no-default-features` build has no `rcdzc` in its closure (`cargo tree`). Realizes the caching win.
7. **Nix** — v-nix lands the `CDZ_COMPILE_BIN` wrapper hunk (+ a `--no-default-features` cdz test job).

(Each slice keeps the DEFAULT (`standalone`) build byte-identical to today, so `main` stays green
throughout; the `!standalone` build is progressively made rcdzc-free, culminating in slice 6.)

## Shared boundary types — the `cadenza-compile-abi` extraction (approach B, operator-approved 2026-08-29)

Slices 2–6 need the `!standalone` cdz arms to build the sidecar request + read the compile result
WITHOUT referencing `rcdzc` types (so slice 6's `rcdzc = { optional = true }` flip drops `rcdzc` from
the closure). Two ways to get there were weighed:

- **(A)** cdz keeps its own cdz-local MIRROR of each boundary type (`Artifact`/`Request`/`Query`/
  `OptLevel`/`Target`/spans/codec), duplicating ~7 types + ~100 use-sites. No new rcdzc dep, but the
  contract now lives in two places that can drift.
- **(B, CHOSEN)** EXTRACT the boundary types into a new dependency-light crate **`cadenza-compile-abi`**
  that BOTH `rcdzc` and `cdz` depend on — ONE source of truth for the sidecar/compile wire. This is the
  compile-boundary analogue of `cadenza-ast` (one source of truth for a cross-process wire format).

The operator approved (B) on 2026-08-29 ("I'm fine to have a compile abi crate"), which makes
`rcdzc → cadenza-compile-abi` the **SECOND sanctioned exception** to rcdzc's COPY-DON'T-DEPEND rule
(the first being `cadenza-ast`), on the same cross-process-wire-contract rationale. v-inference
concurred after auditing the abi/sidecar lane.

**What extracts (v-inference-audited clean-extract set):** `abi.rs` {`Artifact` + `KIND_*` consts,
`CompileOutput`, `Severity`, `FixKind`, `WRAP_HOLE`}; `sidecar.rs` {`Request`, `Query`, the encode/
decode codec (builds on `cadenza_ast::Builder` only)}; `backend::Target`; `opt::OptLevel`; `spans`
{`SpanData`/`SpanTable` + codec}. The compiler IMPLEMENTATIONS behind the boundary — `run_query` (a
query over a live `Db`), `compile`, the backends — STAY in `rcdzc`, which `pub use`s what moves so its
public API (`rcdzc::Target`, `rcdzc::Request`, …) is byte-stable and every consumer path keeps
resolving. The DEFAULT (`standalone`) build stays byte-identical.

**The one orphan-rule snag (v-inference OWNS it):** `abi.rs`'s `Diagnostic`/`DiagnosticFix` carry
conversion methods that couple to rcdzc-internal `diag::Reject`/`diag::Fix` (`Diagnostic::from_reject`,
`DiagnosticFix::from_fix`, `Diagnostic::with_fix(Fix)`). Once those types move to the light crate, the
orphan rule forbids those staying as INHERENT impls in `rcdzc` → they become FREE FUNCTIONS (or a small
`ToAbi` trait) in `rcdzc` (~12 `compile.rs` sites). cdz is UNAFFECTED (it only reads `Diagnostic`
fields, never converts).

**Extraction slice order + ownership:**
- **E1 (LANDED-path):** create `cadenza-compile-abi` with the two pure-std leaf enums `Target` +
  `OptLevel` (zero deps). Registered STANDALONE/inert first — **v-nix owns the flake wiring** (a new
  root workspace member needs a `rootWorkspaceCrates` entry PLUS `test-`/`clippy-` crane entries + a
  `crateClosureAssert` leaf — the xtask-bench pattern; a bare map-line edit REDs the flake eval), so
  v-nix registers the crate; then the `rcdzc → cadenza-compile-abi` dep + re-exports land on top.
- **E2 (solo):** move the Diagnostic-FREE remainder — `Artifact` + `KIND_*`, `Request`/`Query` + codec
  (takes the single `cadenza-ast` dep), `spans` — into the crate (no new flake registration needed once
  E1 is in; extend the `crateClosureAssert` leaf for the `cadenza-ast` dep with v-nix).
- **E3 (with v-inference):** move `CompileOutput` + `Diagnostic`/`DiagnosticFix` + `Severity`/`FixKind`
  + `WRAP_HOLE`, paired with v-inference's orphan-rule refactor.
- Then slices 3–6 above point cdz's `!standalone` arms at `cadenza-compile-abi` and flip `rcdzc`
  optional.

## Invariants to pin in the gate

- Feature-off `cdz compile` output == feature-on `cdz compile` output == `cdz-compile` output, for
  single-file, multi-file package (`--entry`), and imposed-world (`--component-name`) cases.
- Located diagnostics (`path:line:col`) are byte-identical across the boundary (guaranteed by
  passing `spans` artifacts through, but pinned by a reject-case test).
- A `!standalone` build's dependency closure excludes `rcdzc` (slice 6 — a `cargo tree` assertion in
  `cargo xtask check`).
