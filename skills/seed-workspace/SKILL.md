---
name: seed-workspace
description: >-
  How to build, run, test, and gate the Cadenza seed toolchain via `cargo xtask`.
  Read this whenever working in the Rust workspace under implementation/seed/crates
  (cadenza-syntax, rcdzc, cdz-run, cdz-runtime) or in xtask/ — compiling or running
  a .cdz/.sexp program, running the corpus behavior gate, checking library health,
  formatting, round-tripping the syntax surfaces, or inspecting a binary AST.
---

# Driving the Cadenza seed workspace

`cargo xtask` is the ONE interface for everything in this workspace — building, running programs,
and gating. Prefer it over hand-rolled `cargo run -p …` pipelines: it choreographs the real tool
binaries, computes paths, and keeps results consistent. xtask itself pulls in no workspace crate as
a library; it builds and drives `cdz-syntax`, `rcdzc`, and `cdz-run` as processes.

## The crates

- **`cadenza-syntax`** — the front end. Lexer, parser, the two arenas AST + binary codec, printers.
  Binary is `cdz-syntax`. Converts between surfaces: `binary`, `sexpr`, `ml`, and two output-only
  debug views, `debug` (indented tree) and `flat` (arenas dumped literally).
- **`rcdzc`** — the reference compiler: binary AST → a WebAssembly component. Binary is `rcdzc`.
- **`cdz-run`** — the wasmtime host: runs a finished component, resolving the value-heap runtime by
  content address from the store. Binary is `cdz-run`.
- **`cdz-runtime`** — the value-heap runtime, built ONLY for `wasm32-unknown-unknown`. It is
  **excluded from the native workspace**, so a plain `cargo build` skips it — `cargo xtask build`
  and `cargo xtask check` build it explicitly.

The pipeline is `cdz-syntax | rcdzc | cdz-run`; each tool streams stdin→stdout (a `-` arg reads
stdin), so they compose as a real pipe with no temp files.

## The commands

Run from anywhere in the repo (`cargo xtask` resolves the workspace root itself).

| command | what it does |
|---|---|
| `cargo xtask build` | build the value-heap runtime component, content-address it, store it under `target/cadenza-store` |
| `cargo xtask run <file.cdz>` | compile-and-run one program end-to-end; prints the result to stdout |
| `cargo xtask emit <file.cdz> [-o out.wasm]` | compile only — write the component, don't run it |
| `cargo xtask gate [FILE.sexp…]` | run the corpus and grade each case (defaults to all of `spec/semantics/*.sexp`) |
| `cargo xtask check` | omnibus health check: build + test + clippy + wasm-runtime + gate |
| `cargo xtask roundtrip [FILE.sexp…]` | every corpus program must round-trip through the syntax surfaces |
| `cargo xtask fmt [--check] <file…>` | format program files through the printer |
| `cargo xtask bench [--save]` | runtime **allocation benchmark**: gross heap allocs per hot op, diffed against the committed `spec/bench/.alloc-baseline` (regression ⇒ non-zero exit); `--save` records the baseline |

Global `--profile <name>` picks the cargo profile the pipeline tools are built under. It defaults to
**`release-debug`** (optimized — so the ~900-case gate is fast). Pass `--profile dev` for a quick
unoptimized build when iterating on the tools themselves.

## Common tasks

**Run a program:**
```
cargo xtask run path/to/prog.cdz          # → the rendered result on stdout
echo '(module m (def (main) 42) (export main))' > /tmp/p.cdz && cargo xtask run /tmp/p.cdz   # → 42
```

**Is the library healthy?** One command — build, test, clippy, the wasm runtime, and the gate:
```
cargo xtask check
```
Each step's full output is captured to `target/xtask-logs/check-<timestamp>.log`; the console shows
just a ✓ per step. On the first failing step it prints the whole captured log inline (and the path),
so **read that — don't re-run with `| tail`**; the log has everything.

**Run the behavior gate** (compile+run each corpus case, compare to the recorded outcome):
```
cargo xtask gate                          # whole corpus: "N pass, M todo, K fail"
cargo xtask gate spec/semantics/01-literals.sexp   # one file
```
- **pass** — ran and matched the recorded value.
- **todo** — the compiler can't compile it yet (declined), or the expectation needs machinery not
  wired (error-code matching, traps). NOT a failure.
- **fail** — a real disagreement (ran to a wrong value, or ran where a rejection was expected).
  Only a fail makes the gate exit non-zero.

**Debug ONE failing case** — prints its normalized program, expected, and actual:
```
cargo xtask gate --case "substring of the case description"
```

**Guard against a compiler regression** with the committed baseline (`spec/semantics/.gate-baseline`):
```
cargo xtask gate --save     # record the current per-case verdicts as the baseline
cargo xtask gate --check    # fail ONLY if a case that used to pass now doesn't (totals may drift)
```
Refresh the baseline (`--save`) after the compiler gains real ground, so "newly passing" doesn't
grow unbounded. `check` uses `--check` automatically when a baseline exists.

**Track runtime allocation performance** against the committed baseline (`spec/bench/.alloc-baseline`):
```
cargo xtask bench           # measure hot-op heap allocs; fail if any op REGRESSED past baseline+2%
cargo xtask bench --save    # record the current counts as the new baseline (after an improvement)
```
Allocation COUNT — not wall-clock — is the tracked metric: it is identical native↔wasm and
deterministic, so it's a stable regression signal (wall-clock on the native build would measure the
system allocator, not the shipped talc/wasm path). The measurements come from the
`hot_op_allocation_ceilings` test in `cdz-runtime` (one source of truth, shared with its in-crate
allocation-ceiling asserts). After landing a runtime allocation win, run `--save` to record the new
floor; a later change that pushes an op back up fails the bench.

**Inspect a binary AST:**
```
cargo xtask emit prog.cdz -o /tmp/p.wasm                 # get a component to poke at with wasm-tools
cdz-syntax --from binary --to debug /tmp/p.ast           # indented tree of the AST
cdz-syntax --from binary --to flat  /tmp/p.ast           # the arenas literally (leaf pool + structure + root)
```
(`cdz-syntax` is at `target/<profile>/cdz-syntax` after a build; or `cargo run -p cadenza-syntax --bin cdz-syntax --`.)

## Plain cargo still works

For crate-level work, ordinary cargo is fine and often faster to reach for:
```
cargo test -p rcdzc              # test one crate
cargo build --workspace          # native build (does NOT build cdz-runtime — that's wasm-only)
cargo clippy -p cadenza-syntax
```
Use `cargo xtask check` when you want the single green/red signal that also covers the wasm runtime
and the behavior gate — the things a plain `cargo build`/`test` silently misses.

## Gotchas

- **The wasm runtime is invisible to `cargo build`.** `cdz-runtime` is workspace-excluded and builds
  only for `wasm32-unknown-unknown`. `cargo xtask build`/`check` are the only things that build it;
  a green `cargo build --workspace` says nothing about it.
- **The gate's `todo` is honest, not hidden failure.** rcdzc is climbing; most cases are `todo`
  because it can't compile them yet. Diff the FAIL set (or use `gate --check`), not the raw counts —
  pass/todo totals drift as the compiler grows.
- **`.cdz`/`.sexp` files carry the s-expression surface** (`--from sexpr`, the default for `run`).
  The `ml` surface exists but has known round-trip gaps (see `roundtrip`); sexpr is the reliable one.
- **Corpus `input` is normalized on the fly** to the export shape (`(do (def (main) E) (export main))`)
  by `cdz-syntax corpus`; the `.sexp` files themselves are not yet migrated.
