# Workspace CLI Decomposition — Four Focused Tools, xtask As The Orchestrator

**Context:** The seed workspace is driven today by **two** hand-rolled `args.get(N)` CLIs and **four** ad-hoc environment variables, and one crate — `cadenza-seed` — is doing *far* too much: it is simultaneously the wasmtime host, the corpus loader + behavior gate, the probe harness, the compiler-selection shim, AND the CLI dispatch for `behavior-gate`/`emit`/`ignite`/`compile-run`/`component-check`. To get "consistent results" an operator (or an agent, or the `/gate` skill) has to remember to prefix a run with the right combination of `CADENZA_COMPILER=v2 CADENZA_RUNTIME=<fresh wasm> CADENZA_STORE=… CADENZA_TRACE=…` and to hand-position the right positional args — and getting any of it wrong produces a _plausible-looking wrong answer_ (a stale runtime reads as a phantom `runtime missing X` failure; a forgotten `=v2` silently ran the now-deleted old compiler). This is exactly the class of brittleness the project's binding rules already fight elsewhere.

**Goal:** Decompose the workspace into **four focused, single-responsibility tools**, each with a clap-parsed CLI and generated `--help`, and make **`cargo xtask` the one orchestrator** that owns the corpus, the gates, ignition, and codegen. No caller ever sets a `CADENZA_*` env var; no caller hand-positions argv. Each tool does *one* thing:

1. **`cdz-syntax`** — syntax conversion. Reader + **printer** + binary AST codec + value-text rendering + the ML surface. The one home for "text ↔ AST ↔ bytes ↔ ML," with no dependency on the compiler backend or wasmtime.
2. **`cdz-compile`** — `.cdz` (or AST bytes) → wasm component, via `rcdzc`. No wasmtime, no orchestration, no corpus.
3. **`cdz-run`** — the wasmtime host: given a component path, an export to call, and args, it links the value-heap runtime, executes, and returns the rendered result.
4. **`xtask`** — the orchestrator: owns corpus loading, the behavior gate, ignition, the differential/gating logic, and the build-pair codegen. Drives the three tools above by **shelling out** to them.

**Scope:** PLANNING/DESIGN ONLY. No `.rs` file is edited here; this is the map, the target topology, and the ordered plan. This design **coordinates with but does not do** the old-compiler retirement (`DESIGN-retire-old-compiler.md`) and the everything-as-records foundation; where behavior depends on which compiler is live, this document assumes the retirement's endpoint (rcdzc unconditional) and orders the compile-blocked work last.

---

## Executive Summary

- **`cadenza-seed` is dissolved, not extended.** It is doing five jobs; each moves to the tool that owns it. `host.rs` + `runtime_funcs.rs` → **`cdz-run`**; `corpus.rs` + `probe.rs` + gating → **`xtask`**; the reader/codec/render helpers → **`cdz-syntax`**; the `compiler.rs` shim is **deleted** (both `cdz-compile` and xtask call `rcdzc` directly). The overloaded crate stops being a bottleneck where every concern is entangled with wasmtime and argv parsing.
- **Four small clap CLIs replace two hand-rolled ones.** Each binary parses with **clap** (derive API), every knob a typed flag with a computed default, generated `--help`, and a real error on an unknown subcommand/flag — no more `args.get(1).unwrap_or("build")` silent defaults or `iter().position()` flag scans.
- **xtask orchestrates by shelling out.** The behavior gate, ignition, and the differential checks drive `cdz-compile` and `cdz-run` as child processes — this **dogfoods the exact CLI paths** an operator/agent uses, so the gate proves the real tools, not a private in-process path. xtask sets any env the children need; the operator sets none.
- **The s-expr PRINTER is new work and is the linchpin.** Today `rcdzc::ast` has `read`/`encode`/`decode` but **no printer** (Node → text). Shelling out to `cdz-compile <file>` for a corpus case is impossible without either a printer *or* an AST-bytes input mode. This design does both: `cdz-syntax` gains the printer (making "convert between forms" real and enabling human-readable round-trips), and `cdz-compile` accepts `--ast <bytes>` (the build-tool ABI path the gate uses, `build-tool-interface.md`: derivation is `AST bytes → component bytes`).
- **Env vars become internal, then mostly disappear.** Of the four `CADENZA_*` vars, **two are already dead** (`CADENZA_COMPILER`, `CADENZA_STORE` are comment-only — never read via `env::var`). The two live ones get internalized: `cdz-run` takes an explicit `--runtime <path>` (which xtask computes from the store it built); `--trace <filter>` replaces `CADENZA_TRACE`. Env survives only as the transport xtask sets on the `cargo test` child.
- **Fix two breakages the recent moves introduced.** (a) xtask moved from `<seed>/xtask` to `<repo>/xtask`, but `seed_root()` still does `CARGO_MANIFEST_DIR.parent()` — so `build`/`gen-only` now resolve `crates/…` against the wrong root and are broken. (b) `build` step 4 shells `cargo component build` for `cdz-compiler-component`, a **deleted** crate. Both are repaired by moving to computed, workspace-anchored paths.
- **Retire `implementation/stable/`; isolate agents with git worktrees.** The pinned-snapshot dir is already gitignored and all but a leftover binary is deleted. Per-worktree isolation (each worktree its own `target/cadenza-store` + freshly-built runtime) replaces the shared snapshot; xtask's computed store-path default makes it automatic.

**Net effect:** `cargo xtask gate` is the only thing anyone types to gate; `cdz-compile foo.cdz -o foo.wasm` and `cdz-run foo.wasm --call run` are the only things anyone types to compile/run one program. Each tool has focused `--help`. Results are consistent because the one orchestrator computes every path and setting the same way each time.

---

## 1. Current-state inventory — what `cadenza-seed` is doing, and every knob

### 1a. `cadenza-seed`'s five entangled responsibilities

| module | responsibility | moves to |
|---|---|---|
| `host.rs` | wasmtime host: validate, instantiate, compose the value-heap runtime, run an export, render the result, decode a `compile`-export return | **`cdz-run`** (lib + bin) |
| `runtime_funcs.rs` | the generated list of runtime imports the host forwards | **`cdz-run`** (generated into it by xtask) |
| `corpus.rs` | load `.sexp` cases, run each, compare observed vs recorded, render observable values | **`xtask`** |
| `probe.rs` | compile→validate→run one program into a structured `Probe` outcome | **`xtask`** (its gating/dev harness) |
| `compiler.rs` | old↔rcdzc selection shim + byte projection | **deleted** (call `rcdzc` directly) |
| `main.rs` | CLI dispatch for 5 subcommands | **split** across `cdz-compile` / `cdz-run` / `xtask` |

### 1b. The two hand-rolled parsers (both replaced by clap)

| binary | subcommands (dispatch) | parser style |
|---|---|---|
| `xtask/src/main.rs` | `build` (default), `gen-only` | `args.get(1)`; `--store` via `iter().position()` |
| `cadenza-seed/src/main.rs` | `behavior-gate` (default), `emit`, `ignite`, `compile-run`, `component-check` | `args.get(1)`, positional `args.get(2/3)`; `--emit-component` via `position()` then filtered out |

Same anti-pattern in both: silent default on a missing/misspelled subcommand, positional-by-index args, flags discovered by scanning `args` — no validation, no `--help`.

### 1c. The four environment variables

| var | read at | actually read? | disposition |
|---|---|---|---|
| `CADENZA_RUNTIME` | `host.rs:349`, `tests/multi_export.rs` ×6 | **yes** | → `cdz-run --runtime <path>`, computed by xtask; env kept only as the `cargo test` child transport |
| `CADENZA_TRACE` | `main.rs:100` (`--features trace` only) | **yes** | → `--trace <filter>` flag |
| `CADENZA_COMPILER` | — | **no** (comment-only; `compile()` hard-calls `rcdzc::compile_program`) | **delete the vestige** — the `=v2` prose in docs/skills is a no-op that misleads |
| `CADENZA_STORE` | — | **no** (`host.rs:344` names it in prose only) | **make real** → `--store <dir>`, default computed |

> The two dead vars matter *because* they read as live: docs (`DESIGN-records-everywhere-foundation.md:781`, the old `stable/README.md`) still tell callers to set `CADENZA_COMPILER=v2`, which today does nothing — so following the instruction and ignoring it give the same result, eroding trust in every other documented knob.

### 1d. Two breakages from the recent workspace moves

- **Path-root bug.** `xtask/src/main.rs:94` `seed_root()` = `CARGO_MANIFEST_DIR.parent()`, commented "this crate lives at `<seed>/xtask`". xtask now lives at **`<repo>/xtask`**, so `seed_root()` returns `<repo>` and every `seed.join("crates/…")` (:104, :109, :172) and the `repo = seed.parent().parent()` math (:66, :145) point at the wrong directory. `build`/`gen-only` are broken now. The generator *output* paths were already re-pointed to `crates/rcdzc/src/…` (opcodes.rs:184, frame.rs:198, wit_envelope.rs:1437) but they hang off the broken `seed`/`repo`.
- **Deleted-crate build.** `build` step 4 (`main.rs:172`) builds `cdz-compiler-component`, and `component-check` prose (`main.rs:382`) references it. That crate is **deleted** (git `D`). The step is removed (the byte gate returns later re-authored against the Cadenza/rcdzc-emitted component — per `DESIGN-retire-old-compiler.md` §Step 5 + memory).

### 1e. The missing printer + the ml-spike overlap

- `rcdzc::ast` exposes `read`/`read_all`/`read_program` (text→Node) and `encode`/`decode` (Node↔binary bytes), but **no Node→text printer**. The corpus stores each case's `input` as an s-expression, parsed to a `Node`; there is no way to turn it back into `.cdz` text.
- `ml-spike` already implements a *different* printer/reader pair (Node ↔ ML-flavored surface, Pratt precedence) and validates a lossless round-trip against the corpus. That is precisely the "syntax conversion" job — it belongs in `cdz-syntax`, not a throwaway spike.
- The render helpers `string_canonical_text` / `bytes_literal_text` (value → canonical text) are currently **orphaned** mid-surgery (`host.rs:867` still calls the deleted `cdz_compiler::codegen::…`; `corpus.rs:713` calls `rcdzc::codegen::…` which does not resolve — `rcdzc` exposes only `abi`/`ast`/`pipeline`). They are rendering-of-observable-values logic and belong in `cdz-syntax`'s text module (this matches `DESIGN-retire-old-compiler.md` §2's `cdz-syntax` extraction target).

### 1f. `implementation/stable/`

Already gitignored; `README.md`/`SHA256SUMS`/`cdz_runtime.wasm`/`cdz_compiler_component.wasm` deleted (git `D`); only a stale `cadenza-seed` binary remains untracked. Live references are historical (`asks/done/**`, `spec/learnings/**`). Its "point your invocations at `stable/` + `CADENZA_RUNTIME=stable/…`" workflow is the manual-env pattern this design removes.

---

## 2. Target topology — four crates, one orchestrator

```
   cdz-syntax   (lib + bin)        NO backend, NO wasmtime — pure text/AST/bytes/ML + value-text render
   │  ast: Node, read/read_all/read_program, encode/decode, + NEW printer (Node → text)
   │  text: string_canonical_text, bytes_literal_text, escape_byte   (value → canonical text)
   │  ml:   the ML-surface printer/reader (folded in from ml-spike)
   │  bin `cdz-syntax convert`: sexpr ↔ ast-bytes ↔ ml, any direction
   ▼
   rcdzc (lib)  ── the compiler: Ast→Hir→Mir→Lir, real HM ──▶ (op/frame/heap_envelope tables, xtask-generated)
   ▲                                        ▲
   │ depends on cdz-syntax (its front door) │ depends on cdz-syntax (reader)
   │                                        │
   cdz-compile  (bin)                       │
   │  cdz-syntax::read(file) OR --ast bytes → rcdzc::compile_* → component bytes → -o <path>
   │  NO wasmtime.
   │
   cdz-run  (lib + bin)  ── wasmtime host ──  depends on cdz-syntax (render) only; NOT on rcdzc
   │  validate/instantiate/compose-runtime/call-export/render  (was cadenza-seed::host)
   │  bin: cdz-run <component.wasm> --call <export> [--arg …] --runtime <path>
   │
   xtask (bin)  ── the orchestrator; owns the dev-desk oracle (wasm-encoder/wasmparser/wit-parser) ──
      build / gen-only : codegen + build-pair (shells `cargo component build`; writes generated tables)
      gate / behavior-gate / ignite : owns corpus.rs + probe logic; SHELLS OUT to cdz-compile + cdz-run
      depends on cdz-syntax (parse corpus, encode AST for --ast); does NOT depend on cdz-run/cdz-compile as libs
```

**Dependency directions (all one-way, no cycles):**
- `cdz-syntax` is a leaf — depends on neither `rcdzc` nor wasmtime. Its runtime deps are the codec (`ciborium`) and NFC (`unicode-normalization`), both wasm-portable.
- `rcdzc → cdz-syntax` (the reader is rcdzc's front door; matches the retire-doc's extraction).
- `cdz-compile → rcdzc, cdz-syntax`.
- `cdz-run → cdz-syntax` only (wasmtime lives here; **never** depends on `rcdzc` — running a component needs no compiler).
- `xtask → cdz-syntax` (for corpus parse + AST encode). xtask reaches `cdz-compile`/`cdz-run` by **process**, not by lib — so the oracle crates (`wasm-encoder`/`wasmparser`/`wit-parser`) stay confined to xtask and never enter the shipped-compiler graph (`component-abi.md`: "the compiler builds components ITSELF").

---

## 3. The three tool CLIs

### 3a. `cdz-syntax` — syntax conversion

```
cdz-syntax convert <INPUT> [--from sexpr|ast|ml] [--to sexpr|ast|ml] [-o <path>]
   sexpr : canonical s-expression text (.cdz / .sexp)
   ast   : binary AST bytes (the build-tool interchange)
   bin   : (folded from ml-spike) the ML-flavored surface
   --from/--to inferred from extension when omitted; round-trip is lossless.
```

- **New work:** the Node→text **s-expr printer** (`--to sexpr`). Today only `read`/`encode`/`decode` exist. The printer is what makes `convert` real and what lets a human read back an AST-bytes artifact.
- Folds in ml-spike's ML printer/reader as `--from/--to ml`, and keeps its corpus round-trip check as an xtask-driven test.
- Owns the value-text render helpers (`string_canonical_text`, `bytes_literal_text`, `escape_byte`) that both `cdz-run` (rendering a run result) and xtask's corpus oracle (rendering the expected value) need.

### 3b. `cdz-compile` — one program → one component

```
cdz-compile <program.cdz> -o <component.wasm>      # human path: read text via cdz-syntax
cdz-compile --ast <ast.bin> -o <component.wasm>    # ABI path: AST bytes in (build-tool-interface.md)
   [--emit-component-abi]   # emit a `compile : list<u8> -> list<u8>` component (was `compile-run`'s --emit-component)
```

- Pure `input → rcdzc::compile_* → component bytes`. No wasmtime, no run, no corpus. A decline/reject prints the full diagnostics list (`compiler-pipeline.md` §Phases Recover From Errors) and exits non-zero without writing a component.
- The `--ast` mode is what xtask's gate feeds (see §4) — no printer needed on the gate path, though the printer still lands in `cdz-syntax` for human use and interchange.
- Subsumes the `emit`-compile half and the `compile-run --emit-component` build half of today's `cadenza-seed`.

### 3c. `cdz-run` — run a component, return the result

```
cdz-run <component.wasm> --call <export> [--arg <val> …] [--runtime <path>] [--input <bytes>]
   --call     : export to invoke; if omitted, the sole `() -> scalar` entry (host finds it by signature)
   --runtime  : value-heap runtime .wasm to compose (REPLACES CADENZA_RUNTIME); xtask computes+passes it
   --input    : bytes for a `compile : list<u8> -> list<u8>` export (was `compile-run`'s input path)
   prints: the rendered value / trap / suspended / decoded compile-outcome, and exit code
```

- This is today's `host.rs` behind a CLI. It owns wasmtime and the runtime composition; it depends on `cdz-syntax` only for value rendering. Because it never compiles, it never touches `rcdzc`.
- Subsumes the `emit`-run half, `compile-run`'s drive half, and the run side of `ignite`.

---

## 4. `xtask` — the orchestrator

```
cargo xtask <COMMAND>
  build            build runtime → content-address → generate envelope+op+frame pinned to hash →
                   build compiler → store the versioned pair.   [--store <dir>]
  gen-only         regenerate the generated sources from WIT+spec, no runtime build (placeholder hash).
  gate             THE promotion bar: `cargo test` + behavior gate + ignition → one 0-FAIL/exit verdict.
                                                                 [--corpus <dir>] [--bootstrap]
  behavior-gate    just the corpus behavior gate.               [--corpus <dir>] [--trace <f>]
  ignite [program] compile → run → recompile; prove byte-identical reproduction.
  --store <dir>    content-addressed store + runtime source.  [default: <repo>/target/cadenza-store]
```

**How the gate drives the tools (shell-out, dogfooding the real CLIs):**

For each realized corpus case, xtask:
1. parses the case `.sexp` with `cdz-syntax` (in-process, it's a leaf lib) → the input `Node`,
2. `encode`s it to AST bytes and writes a temp file,
3. spawns **`cdz-compile --ast <tmp> -o <tmp.wasm>`**, capturing diagnostics/exit,
4. spawns **`cdz-run <tmp.wasm> --call <entry> --runtime <computed>`**, capturing the rendered outcome,
5. compares the observed outcome against the recorded `output`/`error`/`trap`, using `cdz-syntax`'s render helpers for the expected side.

This is why the printer is not on the critical path for gating (the gate uses `--ast`), but the shell-out still exercises the *actual* `cdz-compile`/`cdz-run` binaries an operator uses — the gate proves the tools, not a private path. xtask sets `--runtime` (and, for the `cargo test` leg, `CADENZA_RUNTIME` on the child); the operator sets nothing.

> Cost note: shelling out is a process per case per stage. For a few-hundred-case corpus this is acceptable and worth the dogfooding; if it becomes the gate's bottleneck, a `cdz-compile`/`cdz-run` *batch* mode (many cases per process) is the escape hatch — recorded here, not built now.

**`component-check` is dropped** (already retired from the gate set, 2026-07-08; its native oracle + wasm wrapper are being deleted). A returning byte-differential re-enters later as a new xtask subcommand against the Cadenza/rcdzc-emitted component.

---

## 5. Sequenced steps

Each step is independently buildable. The seed workspace does **not** compile right now (retire-old-compiler is mid-flight — `cadenza-seed/src/main.rs` still imports the deleted `cdz_compiler`), so steps are ordered to put the compile-blocked integration last; the codegen/path fixes and the leaf-crate extraction are verifiable earlier.

### Step 1 — Repair xtask's path root + drop the deleted-crate build (unblocks `build`/`gen-only`)
Anchor paths from the true repo root (parent of `<repo>/xtask`), derive `seed = <repo>/implementation/seed` explicitly, and delete the fragile double-`.parent()` math (:66/:145). Remove the `cdz-compiler-component` build (:172) and its `component-check` prose (:382). Update the stale `xtask/Cargo.toml` "standalone workspace" header.
*Verifies:* `cargo xtask gen-only` regenerates `crates/rcdzc/src/{op,frame,heap_envelope}.rs` in place, no-op (mtimes unchanged) on re-run; `cargo xtask build` completes the runtime build + store.

### Step 2 — Extract `cdz-syntax` (leaf lib) + write the printer
Create `cdz-syntax` with `ast` (Node + reader + codec, moved as pure code motion), `text` (the three render helpers), and the **new Node→text printer**. Add it to `members`. Re-point `rcdzc`/corpus/probe reader imports at it. Fold `ml-spike`'s ML surface in as `cdz-syntax::ml` (keep its round-trip test).
*Blocked on:* the everything-as-records foundation settling `Node`/reader shape (moving a *stable* `ast.rs` — see retire-doc R1/§5). *Verifies:* `cargo test -p cdz-syntax` (round-trips incl. the new printer) green; `rcdzc` builds against it.

### Step 3 — Stand up `cdz-compile` (bin over `rcdzc` + `cdz-syntax`)
New crate: clap CLI with the `<file>` / `--ast` / `-o` / `--emit-component-abi` surface (§3b), calling `rcdzc::compile_*` directly (no shim).
*Blocked on:* rcdzc unconditional (retirement cutover) so there is one compiler to call. *Verifies:* `cdz-compile examples/answer.cdz -o /tmp/a.wasm` produces a component `cdz-run` (Step 4) validates; `--ast` path matches.

### Step 4 — Stand up `cdz-run` (bin+lib from `cadenza-seed::host`)
Move `host.rs` + `runtime_funcs.rs` into `cdz-run`; add the clap CLI (§3c) with `--runtime`/`--call`/`--arg`/`--input`; replace `env::var("CADENZA_RUNTIME")` with the explicit `--runtime` param (env kept only inside the `cargo test` child).
*Verifies:* `cdz-run /tmp/a.wasm --call run --runtime <fresh>` renders the expected value; a stale/missing runtime is an honest error, not a phantom `runtime missing X`.

### Step 5 — Move corpus + gating into xtask; add `gate`/`behavior-gate`/`ignite`; shell out
Move `corpus.rs` + the probe/gating logic into xtask (in-process `cdz-syntax` for parse/encode). Implement the shell-out loop (§4). `gate` = `cargo test` (xtask sets `CADENZA_RUNTIME` on the child) + behavior gate + ignition → one verdict; `--bootstrap` selects the ignition subset.
*Verifies:* `cargo xtask behavior-gate` reproduces the prior PASS/todo/skip/FAIL tally; `cargo xtask gate` reproduces the three-gate 0-FAIL/exit-0 signal; a seeded regression flips it.

### Step 6 — Delete `cadenza-seed`; retire `stable/`; re-point docs & skills
Delete the dissolved `cadenza-seed` crate (all responsibilities have moved) and drop it from `members`. Delete the leftover `implementation/stable/cadenza-seed` binary and the dir. Re-point every invocation site — `.claude/commands/{gate,ignite}.md`, the `CADENZA_COMPILER=v2 … cargo run -p cadenza-seed` recipes in `DESIGN-records-everywhere-foundation.md` / `SEED-GAPS-FOR-SELF-HOSTING.md`, and the memory operational-traps that name the old invocation — to `cargo xtask …` / `cdz-compile …` / `cdz-run …`. Add the worktree-isolation note (§6).
*Verifies:* `grep -rn "cargo run -p cadenza-seed\|CADENZA_COMPILER\|CADENZA_STORE\|stable/cadenza-seed"` over live files (excluding `asks/done`, `spec/learnings`) is empty; `cargo xtask gate` green.

---

## 6. Retiring `stable/` in favor of git worktrees

`stable/` gave concurrent loop/agent runs a fixed, shared toolchain+runtime to probe against. Git worktrees give that isolation more cleanly:

- Each agent works in its own worktree (the harness offers `isolation: "worktree"`), which has its **own** `target/` — so its `target/cadenza-store` and freshly-built `cdz_runtime.wasm` are private. No two agents share (or stale) one runtime.
- Because xtask computes the store/runtime path from a repo-anchored default, "which runtime" resolves correctly *inside each worktree automatically* — no `CADENZA_RUNTIME=stable/…` prefix, no shared snapshot to keep in sync.
- The reproducibility `stable/` nominally provided is instead the content-addressed store's: a given source+toolchain derives a given hash, and `<store>/<hash>.wasm` is that pinned artifact — per worktree.

Action: delete the leftover `stable/` binary + dir; do **not** re-create the snapshot workflow; document worktree-per-agent in the Step 6 invocation note.

---

## 7. Risk ledger — failure mode → structural prevention

| # | Risk | Prevention |
|---|---|---|
| R1 | The reader is the shared front door; moving `ast.rs` could silently change NFC/escape behavior → alters **string equality** across every consumer. | Step 2 moves `ast.rs` as **pure code motion, zero edits**; keep its unit tests with it; diff old-vs-new. (Mirrors retire-doc R1.) |
| R2 | The new printer is not a true inverse of the reader → `convert` and any human round-trip silently corrupt programs. | The printer lands with a `read(print(x)) == x` (and `read_all`) round-trip test over the whole corpus, alongside ml-spike's existing round-trip. Printer is NOT on the gate's critical path (gate uses `--ast`), so a printer bug can't silently pass the gate. |
| R3 | `cdz-run` accidentally depends on `rcdzc` (or xtask's oracle crates leak into a shipped crate). | Directions are one-way (§2): `cdz-run → cdz-syntax` only; oracle crates declared **only** in xtask. Verify with `cargo tree -p cdz-run` / `-p rcdzc` showing none of wasm-encoder/wasmparser/wit-parser and no rcdzc under cdz-run. |
| R4 | Per-case shell-out makes the gate too slow. | Accepted for a few-hundred-case corpus (dogfooding is the payoff). Escape hatch recorded (§4): a batch mode on `cdz-compile`/`cdz-run`. Not built now. |
| R5 | Internalizing `CADENZA_RUNTIME` breaks the `multi_export.rs` tests that read it directly. | Step 4 keeps the env as the `cargo test` **child** transport (xtask sets it); the parent/CLI path stops reading it. Optional later test migration noted. |
| R6 | The xtask path-root fix (Step 1) misses a `join` elsewhere → codegen writes to the wrong dir / compiles against a stale table. | Step 1 replaces **all** `seed`/`repo` derivations with one explicit anchor; verified by a no-op `gen-only` re-run landing in `crates/rcdzc/src/` with unchanged mtimes. |
| R7 | Deleting `cadenza-seed` before docs/skills re-point leaves `/gate`, `/ignite` invoking a gone binary. | Step 6 does re-point + delete together, gated on the stale-invocation grep being empty. |
| R8 | This work entangles with the unfinished old-compiler retirement (seed doesn't compile now). | Steps 1–2 (path fix + leaf extraction) verify early; Steps 3–6 are explicitly blocked on the retirement cutover and ordered last; both designs share the rcdzc-unconditional endpoint. |
| R9 | `build`-is-default / `behavior-gate`-is-default muscle memory breaks under clap's explicit-subcommand requirement. | Decide per binary in Steps 2/5 (documented default subcommand or required arg); call it out in the Step 6 skill/doc re-point. |

---

## 8. Out of scope (recorded, not planned)

- A **returning byte-differential gate** (against the Cadenza/rcdzc-emitted compiler component) re-enters as a new `xtask` subcommand when that capability lands — not resurrected from the deleted `component-check`/`cdz-compiler-component`.
- **Batch modes** on `cdz-compile`/`cdz-run` (the R4 escape hatch) — only if per-case shell-out becomes the gate bottleneck.
- The **old-compiler retirement** and **everything-as-records foundation** are prerequisites for Steps 2–6, executed under their own plans; this design assumes their endpoints.
