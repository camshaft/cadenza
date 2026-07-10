# Retiring the old `cdz-compiler` crate — dependency inventory & sequenced plan

**Context:** The seed toolchain now has two compilers. The OLD one — `crates/cdz-compiler`, whose `src/codegen.rs` is a single ~895 KB / 16 k-line fused-emit walk — is no longer wanted as a compiler. The NEW one — `crates/rcdzc`, the clean nanopass reference compiler (Ast→Hir→Mir→Lir, real Hindley-Milner) — is the real compiler, selected today via `CADENZA_COMPILER=v2` (`crates/cadenza-seed/src/compiler.rs:23`). The goal is to RETIRE `cdz-compiler`.

It cannot simply be deleted. Three things inside it are still load-bearing:
1. **The s-expression reader + binary AST codec** (`src/ast.rs`) — `rcdzc` has NO reader of its own; it (and `cadenza-seed`, and `ml-spike`) all parse through `cdz_compiler::ast`.
2. **Two rendering helpers** (`src/codegen.rs::string_canonical_text`, `::bytes_literal_text`) used by the host and the corpus oracle to render observable values.
3. **The generated backend tables** (`src/op.rs`, `src/frame.rs`, `src/heap_envelope.rs`) — `rcdzc` `#[path]`-includes these files directly, and `xtask` *writes* them into `cdz-compiler/src/`.

Plus one thing that is retirable but anchors a correctness property: the OLD compiler is `tests.rs`'s **byte-identity oracle** and `compiler.rs`'s **default shipping compiler**.

**Scope:** PLANNING/DESIGN ONLY. No `.rs` file is edited or deleted here; this is the map and the ordered plan. This work executes AFTER the current map-eq hang fix and the everything-as-records foundation land (see §5).

---

## Executive Summary

- **22 `cdz_compiler::` use-sites** across 4 crates (`rcdzc`, `cadenza-seed`, `ml-spike`, `cdz-compiler-component`), plus **3 physical `#[path]` includes** of generated tables and **3 xtask output paths** that target `cdz-compiler/src/`.
- Classification:
  - **(a) AST reader/codec — EXTRACT (must not be deleted):** 13 sites. The single most-depended-on surface.
  - **(b) still-needed helpers — EXTRACT:** 3 sites (`string_canonical_text` ×2, `bytes_literal_text` ×1) + one self-contained private helper (`escape_byte`).
  - **(c) differential-gate oracle / default compiler — RETIRABLE (needs a replacement):** the `codegen::compile_program` / `codegen::compile` calls in `tests.rs`, `compiler.rs`, `probe.rs`, `main.rs`, and `cdz-compiler-component`.
  - **(d) genuinely-dead old-compiler surface — DELETE:** the ~16 k lines of `codegen.rs` minus the two helpers, plus `diagnostics.rs` (no external referent; `rcdzc` uses its own `diag.rs`).
- **Extraction target:** a new leaf crate **`cdz-syntax`** holding `ast` (Node + reader + codec) and a `text` module (the two render helpers). It has no dependency on `rcdzc` or `cadenza-seed`, so no cycle is possible.
- **Generated tables** (`op`/`frame`/`heap_envelope`) move to `rcdzc/src/` (their only surviving consumer), and the three `xtask` generators are re-pointed there. This removes the last reason `rcdzc` depends on `cdz-compiler`.
- **Differential-gate recommendation:** replace the against-the-old-compiler byte oracle with a **committed golden-bytes snapshot** (option a), and keep the **corpus value gate** (option b) as the semantic anchor. The "second-implementation independence" the oracle nominally provides is *already stale* — the old compiler is frozen and about to be deleted — so it degenerates into "rcdzc matches a fixed reference," which a golden snapshot captures more cheaply and over a broader case set. No real correctness property is lost.

---

## 1. Complete dependency inventory

`grep -rn "cdz_compiler::" crates/` yields 22 use-sites (comments included, since some carry the only prose describing the dependency). Each classified `(a)`/`(b)`/`(c)`/`(d)` per the task's taxonomy.

### (a) AST reader / codec — MUST be extracted, not deleted

The reader (`ast::read`, `read_all`, `read_program`), the codec (`ast::encode`, `ast::decode`), and the `ast::Node` type itself. `rcdzc` has no reader — this is its front door.

| file:line | use | note |
|---|---|---|
| `crates/rcdzc/src/pipeline.rs:12` | `use cdz_compiler::ast::{self, Node};` | rcdzc's decode entry (`ast::decode` at pipeline.rs:92) |
| `crates/rcdzc/src/resolve.rs:17` | `use cdz_compiler::ast::Node;` | resolve pass walks `Node` |
| `crates/rcdzc/src/lib.rs:23` | doc: "`cdz_compiler::ast` (Node + codec)" | prose; the code dep is via pipeline/resolve |
| `crates/rcdzc/src/tests.rs:2` | `use cdz_compiler::ast;` | `ast::read`/`encode` build test inputs |
| `crates/rcdzc/src/tests.rs:14` | `cdz_compiler::ast::Node` (return type of `program_v2`) | test helper |
| `crates/cadenza-seed/src/compiler.rs:18` | `use cdz_compiler::ast::Node;` | the compiler-selection shim's input type |
| `crates/cadenza-seed/src/corpus.rs:7` | `use cdz_compiler::ast::{self, Node};` | `ast::read_all` parses corpus `.sexp` files |
| `crates/cadenza-seed/src/main.rs:24` | `use cdz_compiler::{ast, codegen};` (the `ast` half) | CLI reads programs (`ast::read`/`read_program`/`encode` at main.rs:161,234,273,365,413) |
| `crates/cadenza-seed/src/probe.rs:7` | `use cdz_compiler::{ast, codegen};` (the `ast` half) | `ast::read` at probe.rs:30,81,107 |
| `crates/cadenza-seed/src/probe.rs:38` | `node: &cdz_compiler::Node` | uses the `lib.rs` re-export `cdz_compiler::Node` (not `::ast::Node`) — repoint carefully |
| `crates/cadenza-seed/tests/multi_export.rs:9` | `use cdz_compiler::ast;` | integration test parses source |
| `crates/ml-spike/src/corpus_test.rs:6` | `use cdz_compiler::ast::{self, Node};` | `ast::read_all` round-trips corpus inputs |
| `crates/ml-spike/src/main.rs:15` | `use cdz_compiler::ast::{self, Node};` | ML-surface spike prints `Node` |

`ast.rs` is a clean extraction target: its only imports are `std::fmt` (ast.rs:14) and `super::*` in its own test module (ast.rs:679). Its runtime deps are `ciborium` (CBOR codec, `encode`/`decode` at ast.rs:503/531) and `unicode-normalization` (NFC on read) — both already declared in `cdz-compiler/Cargo.toml:21,25` and both wasm-portable.

### (b) Still-needed helpers — EXTRACT (they live in codegen.rs, which is being deleted)

| file:line | use | target symbol |
|---|---|---|
| `crates/cadenza-seed/src/host.rs:867` | `cdz_compiler::codegen::string_canonical_text(s)` | `codegen.rs:13752` |
| `crates/cadenza-seed/src/corpus.rs:673` | `codegen::string_canonical_text(s)` | `codegen.rs:13752` |
| `crates/cadenza-seed/src/corpus.rs:692` | `cdz_compiler::codegen::bytes_literal_text(&bytes)` | `codegen.rs:13681` |

Both helpers are **fully self-contained** (verified): `string_canonical_text` (`codegen.rs:13752-13769`) is a pure `&str → String` closed-escape renderer; `bytes_literal_text` (`codegen.rs:13681-13690`) calls one private helper `escape_byte` (`codegen.rs:13660-13676`), which is itself pure. None touch `codegen`'s compiler state (`CVal`, emit context, etc.). They are rendering-of-observable-values logic and belong with the AST/text layer, not the compiler. Move all three (`string_canonical_text`, `bytes_literal_text`, `escape_byte`) into `cdz-syntax`'s `text` module.

> Note: `codegen.rs`'s own `canonical_text` (codegen.rs:13771) also calls `string_canonical_text`, but `canonical_text` is being deleted with the rest of `codegen.rs`, so that internal referent evaporates.

### (c) Differential-gate oracle & default compiler — RETIRABLE (see §3 for the replacement)

| file:line | use | role |
|---|---|---|
| `crates/rcdzc/src/tests.rs:33` | `cdz_compiler::codegen::compile_program(&oracle_node)` | **THE byte-identity oracle** (`scalar_entry_byte_identical_to_oracle`, tests.rs:24-36) |
| `crates/cadenza-seed/src/compiler.rs:19` | `use cdz_compiler::codegen::{self, Decline};` | the **default** compiler branch |
| `crates/cadenza-seed/src/compiler.rs:36` | `codegen::compile_program(node)` | dispatched when `CADENZA_COMPILER != v2` |
| `crates/cadenza-seed/src/compiler.rs:56` | `Result<Vec<u8>, Decline>` byte projection | `Decline` type re-exported from codegen |
| `crates/cadenza-seed/src/probe.rs:40,85,109` | `codegen::compile_program(...)` | probe harness compiles with the OLD compiler directly |
| `crates/cadenza-seed/src/main.rs:241` | `codegen::compile_program` (`run_emit`) | `emit` subcommand |
| `crates/cadenza-seed/src/main.rs:366` | `codegen::compile_program` (ignite) | `ignite` subcommand |
| `crates/cadenza-seed/src/main.rs:411,533,546` | `codegen::compile_program`, `codegen::Decline` (component-check native oracle) | `component-check` subcommand — **already RETIRED from the gate set** (per project memory, 2026-07-08) |
| `crates/cdz-compiler-component/src/lib.rs:18` | `cdz_compiler::codegen::compile(&ast)` | the OLD compiler wrapped as a wasm component — the thing that USED to feed `component-check` |

Observation: several of these (probe.rs, main.rs `emit`) call the OLD `codegen` *directly* rather than through the `compiler.rs` selection shim — so they always use the old compiler regardless of `CADENZA_COMPILER`. Retirement must re-point them at `rcdzc` (via the shim or directly), which is a behavior improvement, not just a mechanical swap.

### (d) Genuinely-dead old-compiler surface — DELETE

- **`crates/cdz-compiler/src/codegen.rs`** minus the two extracted helpers — the ~16 k-line fused emitter. No external referent survives once (a)+(b)+(c) are handled.
- **`crates/cdz-compiler/src/diagnostics.rs`** — grep shows **zero** `cdz_compiler::diagnostics` use-sites; `rcdzc` uses its own `diag.rs`. Dead to all consumers; only `codegen.rs` referenced it internally. Deletes with `codegen.rs`.
- **`crates/cdz-compiler-component/`** (whole crate) — wraps the OLD compiler as a wasm component. Its consumer, `component-check`, is retired from the gate. Delete it (or, per memory, it "returns as the real byte gate when the CADENZA-authored compiler emits the component" — i.e. re-authored against `rcdzc`/`cdzc.cdz`, not resurrected against the old core).

### Physical includes & generator outputs (NOT `cdz_compiler::` paths, but part of the blast radius)

These do not appear in the `grep` because they are `#[path]` file includes and filesystem writes, but they are the reason the crate directory cannot be `rm`'d until they move:

| location | reference | note |
|---|---|---|
| `crates/rcdzc/src/lib.rs:52-53` | `#[path = "../../cdz-compiler/src/frame.rs"] mod frame;` | generated table |
| `crates/rcdzc/src/lib.rs:54-55` | `#[path = "../../cdz-compiler/src/op.rs"] mod op;` | generated table |
| `crates/rcdzc/src/lib.rs:63-65` | `#[path = "../../cdz-compiler/src/heap_envelope.rs"] mod heap_envelope;` | generated table |
| `xtask/src/opcodes.rs:173` | writes `crates/cdz-compiler/src/op.rs` | generator output path |
| `xtask/src/frame.rs:146` | writes `crates/cdz-compiler/src/frame.rs` | generator output path |
| `xtask/src/wit_envelope.rs:1237` | writes `crates/cdz-compiler/src/heap_envelope.rs` | generator output path |

`op.rs`, `frame.rs`, `heap_envelope.rs` have **no imports at all** (verified) — they are self-contained constant tables — so they relocate cleanly. `xtask/src/wit_envelope.rs:1238` also writes `crates/cadenza-seed/src/runtime_funcs.rs`, which is unaffected (stays where it is).

### Cargo dependency edges to sever

`cdz-compiler = { path = "../cdz-compiler" }` appears in:
- `crates/rcdzc/Cargo.toml:27`
- `crates/cadenza-seed/Cargo.toml:18` (+ the `trace` feature passthrough `cdz-compiler/trace` at line 39)
- `crates/ml-spike/Cargo.toml:7`
- `crates/cdz-compiler-component/Cargo.toml:21`

And in the workspace: `crates/cdz-compiler` is a member (`Cargo.toml` `members`), and `crates/cdz-compiler-component` is in `exclude`.

---

## 2. The extraction — new crate `cdz-syntax`

### Shape

A new **leaf** crate `crates/cdz-syntax` (name chosen over `cdz-ast` because it will hold both the syntax tree *and* the value-render helpers, which are text/syntax concerns, not AST-node concerns):

```
crates/cdz-syntax/
  Cargo.toml          # deps: ciborium, unicode-normalization (moved from cdz-compiler)
  src/
    lib.rs            # pub mod ast; pub mod text; pub use ast::Node;
    ast.rs            # MOVED verbatim from cdz-compiler/src/ast.rs (pure code motion)
    text.rs           # string_canonical_text, bytes_literal_text, escape_byte
                      #   (moved from codegen.rs:13660-13690, 13752-13769)
```

`lib.rs` re-exports `pub use ast::Node;` so the one `cdz_compiler::Node` (via re-export) site — `probe.rs:38` — maps to `cdz_syntax::Node`.

### Re-pointing

- **rcdzc:** `use cdz_compiler::ast::…` → `use cdz_syntax::ast::…` in `pipeline.rs:12`, `resolve.rs:17`, `tests.rs:2,14`. Add `cdz-syntax` to `rcdzc/Cargo.toml`.
- **cadenza-seed:** `use cdz_compiler::ast::…` → `use cdz_syntax::ast::…` in `compiler.rs:18`, `corpus.rs:7`, `main.rs:24`, `probe.rs:7`, `multi_export.rs:9`, and `probe.rs:38` (`cdz_compiler::Node` → `cdz_syntax::Node`). The helper calls `codegen::string_canonical_text` / `::bytes_literal_text` → `cdz_syntax::text::…` in `host.rs:867`, `corpus.rs:673,692`. Add `cdz-syntax` to `cadenza-seed/Cargo.toml`.
- **ml-spike:** `use cdz_compiler::ast::…` → `use cdz_syntax::ast::…` in `corpus_test.rs:6`, `main.rs:15`. Swap the crate dep.

### Generated tables

Move `op.rs`, `frame.rs`, `heap_envelope.rs` from `cdz-compiler/src/` into **`rcdzc/src/`** (rcdzc is their only surviving consumer). Then:
- `rcdzc/src/lib.rs:52-65`: change the `#[path]` includes to plain `mod op; mod frame; mod heap_envelope;` (files now local).
- `xtask/src/opcodes.rs:173`, `xtask/src/frame.rs:146`, `xtask/src/wit_envelope.rs:1237`: change output paths from `crates/cdz-compiler/src/…` to `crates/rcdzc/src/…`. Preserve the `@generated`/`write_if_changed` discipline (`xtask/src/main.rs:25`) so a no-op regen doesn't bump mtimes.

(Alternative considered: put the tables in `cdz-syntax`. Rejected — they are backend/emit constants, not syntax; putting them in a `syntax` crate muddies the leaf's charter and makes `ml-spike` transitively pull emit tables it doesn't use.)

### After extraction, does `codegen.rs` have any referent?

No. Once (a) reader → `cdz-syntax`, (b) helpers → `cdz-syntax::text`, (c) oracle/default replaced (§3, §4), and the tables relocated, nothing outside `cdz-compiler` names `cdz_compiler::codegen` or `cdz_compiler::ast`, and nothing `#[path]`-includes its `src/`. `codegen.rs` + `diagnostics.rs` + the `cdz-compiler` crate become deletable.

### Circular-dependency risk

None. `cdz-syntax` depends only on `ciborium` + `unicode-normalization`. The edges become `rcdzc → cdz-syntax`, `cadenza-seed → cdz-syntax + rcdzc`, `ml-spike → cdz-syntax`. A leaf cannot close a cycle.

---

## 3. THE KEY DESIGN QUESTION — replacing the differential gate

### What exists today

`crates/rcdzc/src/tests.rs::scalar_entry_byte_identical_to_oracle` (tests.rs:24-36) compiles four scalar-integer programs (`42, 7, 0, 300`) with the OLD compiler (`cdz_compiler::codegen::compile_program`, tests.rs:33) and asserts `rcdzc`'s bytes are **byte-identical**. This is the byte-identity correctness anchor. A sibling test, `scalar_run_42_is_89_bytes` (tests.rs:42-46), already pins one output as a **frozen constant (89 bytes)** and its own comment (tests.rs:39) notes it "catches a frame-segment regression *even if the oracle drifts*" — the design already anticipates the oracle going away.

(The larger `component-check` differential in `main.rs:390-528` — native-vs-wasm-component agreement — is a separate mechanism and is already **retired from the gate set** per project memory, so it is not the anchor under discussion.)

### What property byte-identity-vs-old-compiler actually buys

Two *independently authored* compilers emitting the identical byte string is strong evidence neither has a codegen bug on that path. But that independence is **already stale**:
- The old compiler is **frozen** — no longer developed. It is not an independent second opinion evolving alongside rcdzc; it is a fixed reference.
- It is about to be **deleted**, so the oracle cannot be recomputed regardless.
- Its coverage is 4 tiny scalar programs — a sliver of rcdzc's surface.
- If both compilers shared a bug, byte-identity would not catch it anyway.

So the live value of the oracle today is really just "rcdzc matches a fixed reference on 4 cases."

### Options

**(a) Frozen golden-bytes snapshot.** Record rcdzc's current output for the anchor cases as committed byte constants; assert stability on every run. Preserves exact byte-regression detection, extends trivially to *more* anchor cases (e.g. an arithmetic path, a heap-compound path), self-contained, zero dependency on the old crate. Loses the (already-stale) "second implementation" flavor. The existing 89-byte test is already exactly this pattern.

**(b) Drop byte-identity; rely on the corpus VALUE oracle.** The behavior gate compiles every realized corpus case under `CADENZA_COMPILER=v2`, runs the component, and checks the observable *value* against the recorded result. This is the real semantic correctness anchor. It checks *what the program computes*, not *the exact bytes emitted* — so it would not notice byte-level churn, nondeterminism, or a size regression that still runs correctly (the "soft-agree" category).

**(c) rcdzc self-consistency.** Idempotence / round-trip: `ast::decode(ast::encode(n)) == n`, and recompilation is byte-stable (the `ignite` subcommand already does compile→content-address→recompile→assert-identical). Checks determinism, not correctness against any reference.

### Trade-off analysis & recommendation

**Recommend (a) + keep (b); optionally keep (c) via `ignite`.**

- Convert `scalar_entry_byte_identical_to_oracle` into a **golden-bytes assertion**: capture today's rcdzc output for the anchor set as committed constants (`let expected: &[u8] = &[…];`) and assert equality. Fold in the existing 89-byte anchor as one entry, and add one non-scalar anchor (an arithmetic and a heap-compound case) so the golden set spans more of the emitter than the old 4-scalar oracle did.
- Keep the **corpus value gate** as the semantic oracle — it is, and remains, the correctness anchor.
- Keep **`ignite`**'s recompile-is-identical check as the determinism guard.

**What is lost:** the "byte-identical to a second, independently-authored implementation" property. **Why it's acceptable:** that property was already forfeited the moment rcdzc became the shipping compiler and the old one froze — a frozen reference is not an independent check, it is a snapshot, and a golden snapshot *is* a snapshot, captured more cheaply and over a broader, extensible case set. A golden snapshot cannot catch a *current* rcdzc bug (it is authored from rcdzc's output), but neither can the frozen old-compiler oracle catch a bug the two share — and forward regression detection (the actual job) is identical between them. Semantic correctness is carried by the corpus value gate; structural validity by the `wasm-tools` validation already run in `probe`/`host`. **No real correctness property is lost.**

---

## 4. Blast radius + sequenced steps

Each step is independently buildable and gate-verifiable. Verification at every step = workspace `cargo build` + `cargo test` + behavior gate green under `CADENZA_COMPILER=v2` (the terminating-and-green precondition from §5 makes "gate green" a usable checkpoint).

### Step 0 — Preconditions (see §5)
Map-eq hang fixed (gate terminates) and everything-as-records foundation landed (Node/reader shape stable). Nothing moves yet.
*Breaks:* nothing. *Verifies:* gate runs to completion and is green — the baseline every later step is diffed against.

### Step 1 — Create `cdz-syntax`, move the reader + helpers
Move `ast.rs` → `cdz-syntax/src/ast.rs` (pure code motion, **no edits** — see risk R1); add `text.rs` with the three helpers. Add `cdz-syntax` to the workspace `members`. Re-point every reader/helper import (§2). For this step, `cdz-compiler` can keep a shim (`pub use cdz_syntax::ast::*;` in its `ast.rs`; `codegen.rs`'s helper calls point at `cdz_syntax::text`) OR take a dep on `cdz-syntax` — either keeps `codegen.rs` compiling while it still exists.
*Breaks:* a missed import site; the `cdz_compiler::Node` re-export at `probe.rs:38` (maps to `cdz_syntax::Node`, not `::ast::Node`). *Verifies:* full build + gate green.

### Step 2 — Relocate the generated tables
Move `op.rs`, `frame.rs`, `heap_envelope.rs` into `rcdzc/src/`; change `rcdzc/src/lib.rs:52-65` from `#[path]` includes to local `mod`s; re-point the three xtask output paths (§2). Run `xtask build` (or `gen-only`) and confirm it regenerates to the new location and leaves mtimes alone on a no-op.
*Breaks:* stale generated files if an xtask path is missed (compiler would compile against an out-of-date table); the `write_if_changed` no-op contract if the header/path logic is fumbled. *Verifies:* `xtask build` regenerates in place + build + gate green.

### Step 3 — Replace the differential-gate oracle
Convert `scalar_entry_byte_identical_to_oracle` (tests.rs:24-36) to a golden-bytes snapshot (§3); this removes the last `cdz_compiler::codegen::compile_program` reference in `tests.rs:33`. Add the extra golden anchors.
*Breaks:* golden constants must be captured AFTER the records foundation (Step 0) or they churn (see risk R3). *Verifies:* `cargo test -p rcdzc` green.

### Step 4 — Retire the old-compiler dispatch and direct calls
- `compiler.rs`: make `rcdzc` unconditional — drop the `use_v2` branch (compiler.rs:31-51), the `use cdz_compiler::codegen::{self, Decline}` (compiler.rs:19), and replace the `Decline`-typed byte projection (compiler.rs:56) with rcdzc's `Diagnostic`-based one.
- `probe.rs`: re-point `codegen::compile_program` (probe.rs:40,85,109) at the `cadenza_seed::compiler` shim / `rcdzc`.
- `main.rs`: re-point `emit` (main.rs:241) and `ignite` (main.rs:366) at the shim. Decide `component-check` (main.rs:390-528, uses `codegen::compile_program` + `codegen::Decline`): since it is retired from the gate, either delete the subcommand or re-point its "native" oracle at rcdzc (making it an rcdzc-vs-rcdzc-component check).
After this, no `codegen::` referent survives outside `cdz-compiler` itself.
*Breaks:* `codegen::Decline` type used in signatures (`main.rs:533,546`, `compiler.rs:56`) — must be replaced with rcdzc's byte-only projection type; probe/emit previously *always* used the old compiler, so their observed outputs may change (this is the intended cutover). *Verifies:* build + gate green.

### Step 5 — Delete `cdz-compiler-component`
Remove the crate directory and drop it from the workspace `exclude` list. (Per memory, the wasm byte gate returns later, re-authored against the Cadenza-authored / rcdzc-emitted component — not this old wrapper.)
*Breaks:* any `xtask` step that builds `cdz-compiler-component` (`xtask/src/main.rs:172`) — remove/replace it. *Verifies:* `xtask build` completes; build + gate green.

### Step 6 — Delete the old crate and sever the deps
Delete `codegen.rs` + `diagnostics.rs`, delete the `cdz-compiler` crate, remove it from workspace `members`, and drop the `cdz-compiler = { path = … }` dep from `rcdzc/Cargo.toml:27`, `cadenza-seed/Cargo.toml:18`, `ml-spike/Cargo.toml:7`. Handle the **`trace` feature**: `cadenza-seed/Cargo.toml:39`'s `trace = ["cdz-compiler/trace", …]` points at codegen's decline/reject instrumentation, which is being deleted — either drop the `cdz-compiler/trace` leg (keep the native subscriber for rcdzc's own future tracing) or remove the passthrough entirely. `cdz-compiler/Cargo.toml`'s `ciborium`/`unicode-normalization`/`tracing` deps move to `cdz-syntax` (the first two) or drop (tracing).
*Breaks:* the `trace` feature wiring; any lingering `cdz_compiler` path. *Verifies:* final full `cargo build` + `cargo test` (all crates) + behavior gate green under `CADENZA_COMPILER=v2`.

---

## 5. Sequencing note — runs AFTER map-eq fix and everything-as-records

This retirement is **gated on two prior landings**, and the reader-extraction interacts with the second:

1. **Map-eq hang fix must land first.** Every step above is verified by "behavior gate green." If the gate *hangs* (the current map-eq issue), there is no usable green checkpoint — every step's verification is blocked, and a regression cannot be distinguished from the pre-existing hang. Fix the hang so the gate *terminates and is green* before starting Step 1.

2. **Everything-as-records foundation must land first — and it directly touches the extraction surface.** The foundation reshapes the value/type model around records, which is very likely to change **`ast::Node`** and/or the **reader** (new record syntax, altered canonical forms, possibly different codec bytes). Two consequences:
   - **Do the `cdz-syntax` extraction (Step 1) AFTER the foundation settles the `Node`/reader shape.** Moving `ast.rs` first would make the foundation work land its `Node` edits in the *new* crate mid-flight, and the pure-code-motion guarantee (risk R1) would be lost to a moving target. Move a *stable* `ast.rs`.
   - **Capture the golden-bytes snapshot (Step 3) AFTER the foundation.** If the record model changes the emitted component layout, golden constants captured earlier would immediately churn. Freeze them against post-foundation output.

In short: land map-eq (so the gate is a usable oracle) and everything-as-records (so `Node`, the reader, and the byte layout are stable), *then* extract, *then* snapshot, *then* delete.

---

## 6. Risk ledger — failure mode → structural prevention

| # | Risk | Prevention |
|---|---|---|
| R1 | The reader is the shared front door; a subtle NFC-normalization or escape behavior change would silently alter **string equality** across every consumer (ast.rs doc: "Parsing And Printing Are Not In The Compiler's Trusted Path" — but equality *is* observable). | Move `ast.rs` as **pure code motion, zero edits**. Diff old-vs-new byte-for-byte. Keep its unit tests (ast.rs:679) with it. |
| R2 | A missed `cdz_compiler::` site (esp. the `cdz_compiler::Node` re-export at probe.rs:38, which is NOT `::ast::Node`) leaves a dangling path after the crate is deleted. | The 22-site table in §1 is the checklist; Step 6 (delete) only proceeds when `grep -rn "cdz_compiler" crates/` is empty. |
| R3 | Golden bytes captured before the records foundation churn immediately. | §5 orders snapshot (Step 3) after the foundation; Step 0 is the stable baseline. |
| R4 | xtask still writes tables to the old `cdz-compiler/src/` path → stale/compile against out-of-date tables, or `write_if_changed` mtime contract broken. | Step 2 re-points all three generators *and* runs `xtask build` to confirm regeneration lands in `rcdzc/src/` and is a no-op on re-run. |
| R5 | The `trace` feature (`cadenza-seed/Cargo.toml:39` → `cdz-compiler/trace`) points at deleted codegen instrumentation. | Step 6 explicitly resolves the passthrough (drop the leg or the feature). |
| R6 | Losing byte-identity-vs-a-second-implementation. | Accepted (§3): the independence was already stale (old compiler frozen). Mitigated by golden snapshot (broader coverage) + corpus value gate (semantics) + `wasm-tools` validity check + `ignite` determinism. |
| R7 | `component-check` (main.rs) and `cdz-compiler-component` are currently-dormant cross-checks; deleting them removes a latent capability. | Per memory, the wasm byte gate *returns* re-authored against the Cadenza/rcdzc-emitted component — the retirement removes the *old-core* wrapper, not the concept. Note in the commit that follows. |
| R8 | `probe`/`emit` previously *always* used the old compiler (they call `codegen` directly, not the shim); cutting them to rcdzc changes their observed output. | Intended cutover, not a regression — but call it out so a diff in probe/emit output is expected at Step 4, not investigated as a bug. |
