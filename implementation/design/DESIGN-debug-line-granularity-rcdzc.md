# Design — per-statement/expression DWARF line granularity (the `select`-layer coverage problem)

**Author:** compiler backend. **Audience:** whoever makes a Cadenza `.wasm` step *line-by-line* in a
debugger (not just stop at function entry). **Status:** DESIGN ONLY — one attempt made and reverted
(see §2), nothing landed. Builds on the shipped DWARF track (`DESIGN-debug-info-rcdzc.md`: D0–D3, Mode
S, CLI targets, auto-spans — all on `spec`). Line references are landmarks at `4471824`.

The shipped DWARF is **function-granularity**: `.debug_line` has one row per function (at its entry),
so a debugger stops at the function and single-steps *instructions* but the "current line" never moves.
The goal here is **per-statement/expression rows** — each source construct maps to the code offset
where its evaluation begins, so "step" advances through the source the way a developer expects.

---

## 1. The corrected diagnosis — this is NOT an IR-erasure problem

An earlier note (and the first attempt) framed this as "the compiler's ANF/inlining erases source
structure, so attribution must move to `lower`." **That framing is wrong, and the code proves it:**

1. **`Core` is a column over the AST's own `StructId`** (`core.rs:4`: "`core_of(id)` fills one node's
   core form, referencing children by their AST `StructId`"). Every `Core` node keeps the source
   occurrence it came from — `Core::If { cond, then_, else_ }`, `Core::Arith { lhs, rhs }`, etc. are
   **`StructId` fields**, not a fresh id space (`core.rs:112`, `:180`, …). Identity is intact through
   lowering.

2. **Inlining does not destroy the inlined node's id.** When `lower::compute` follows a ref
   (`lower.rs:84`, `Resolved::Ref { value } => core_of(db, value)`), the inlined expression's own
   `StructId` (`value`) is right there — the substituted sub-expression still has a real source
   occurrence. A single-use `let` is copy-propagated, but the *value expression* it propagated keeps
   its id.

3. **`select`'s emit helpers already receive the child `StructId`s.** `emit_checked_arith(…, lhs:
   StructId, rhs: StructId, …)` (`select.rs`) has both operand occurrences in hand. So does
   `emit_operand`, `emit_branch`, `emit_div_rem`, … The source ids are threaded all the way into the
   leaf emitters.

**So the real problem is coverage, not erasure:** the source `StructId` reaches every emit point, but
almost none of them *record a line-table marker* at the code offset they emit to. Attribution belongs
in `select` (where code offsets are born), not `lower` (which has no code offsets at all — it produces
`Core`, not `Lir`). Moving it to `lower` would be *impossible*: `lower` cannot know a byte offset.

---

## 2. Why the first attempt failed (the concrete lesson)

The first attempt (reverted) added an `Emit` wrapper (a `Vec<Lir>` + a `lines: Vec<(u32 index,
StructId)>`, `DerefMut` to the vec so the ~294 `out.push` sites were untouched) and recorded a marker
**only at two places**: `emit()`'s entry and the `Core::Let` binding boundaries.

It produced ONE row for a multi-line body. Two reasons, both now understood:

- **`Core::Let` almost never survives.** A scalar single-use binding is copy-propagated
  (`lower.rs:74–86`), so `(let ((b …)(c …)) (+ c 3))` lowers to one `Core::Arith` — the `Let` arm never
  runs. (A *multi-use* scalar binding does survive — verified `(let ((x (+ a 1))) (+ x x))` keeps
  `Core::Let` — but that is the uncommon case.)
- **Marking at `emit()` entry misses the helpers.** The arithmetic/comparison/call operands are emitted
  by `emit_checked_arith` / `emit_operand` / `emit_mul_pow2_as_shift`, which emit a `Param`/`ConstInt`
  operand **inline** without re-entering `emit(child)` — so the child's `StructId` never reached the
  one marker point at `emit()`'s top. Only the outermost node (the root `Arith`) recorded a marker.

**The lesson:** marker placement must be **comprehensive across the `StructId`-consuming emit points**,
not just `emit()`'s entry. The infrastructure (the `Emit` wrapper, the `serialize::instr_offsets`
per-`Lir` byte offsets, the multi-row `.debug_line` program, the `peephole` index-remap) was all built
and *correct* — it just had nothing to record because the markers were too sparse.

---

## 3. The plan — comprehensive markers via the `Emit` wrapper

Reinstate the (reverted) infrastructure, then place markers at **every emit point that consumes a
source `StructId`**, so each source construct's first instruction is attributed.

### 3.1 The carrier — an `Emit` wrapper (unchanged from the reverted attempt)

```rust
pub struct Emit { code: Vec<Lir>, lines: Vec<(u32, StructId)> }
impl Deref/DerefMut for Emit → Vec<Lir>   // out.push(...) etc. unchanged at ~294 sites
impl Emit { fn mark(&mut self, id: StructId) { /* record (code.len(), id), dedup same offset */ } }
```

Swap the 28 `out: &mut Vec<Lir>` signatures to `out: &mut Emit` (a pure, sed-able type rename — all 28
are the identical string; no closure captures `out`; every method used is `push`/`contains`/`last`,
covered by `DerefMut`). `select_function_of` builds an `Emit`, and after `peephole_emit` (the
index-remapping peephole, §3.3) stores `code.lines` into a new `SelectedFunc.stmt_lines`.

### 3.2 Comprehensive marker placement (the part the first attempt missed)

Mark at the **entry of every `StructId`-parameterized emit function**, not just `emit()`. Concretely,
add `out.mark(id)` (guarded to `db.is_user_node(id)`) at the top of each of:
`emit`, `emit_tail`, `emit_operand`, `emit_branch`, `emit_checked_arith` (mark `lhs` then, after
emitting it, `rhs` — each operand is a construct), `emit_div_rem`, `emit_shift`, `emit_mul_pow2_as_shift`,
`emit_call_args` (per arg), the match-arm emitters (per arm body), and the `Core::Let` binding loop
(per binding value + body). The rule: **wherever a distinct source construct's evaluation begins, mark
it.** A helper that emits a *single* construct marks once at entry; a helper that emits *several*
(operands, args, arms) marks before each.

Over-marking is cheap and self-correcting: `Emit::mark` dedups a repeated offset (keep the first — the
outer construct's line), and the backend (`dwarf_funcs_for`) collapses consecutive same-*line* rows to
keep only line **transitions**. So the line table ends up with one row per source line the code visits,
in address order — exactly what a debugger wants.

### 3.3 The `peephole` hazard (already solved in the attempt)

`peephole` fuses `local.set N; local.get N` → `local.tee N`, shifting every later instruction index
down by one. So the line map's indices must be remapped: `peephole_emit` builds an `old→new` index map
as it walks (both fused instructions map to the single `tee`) and rewrites each `lines` entry. This
code was written and correct in the attempt; reuse it verbatim.

### 3.4 Lir-index → byte-offset (already solved)

`stmt_lines` is in `Lir`-index space; DWARF needs byte offsets. `serialize::instr_offsets(f, imports)`
(written in the attempt) replays `code_entry`'s exact byte layout and returns the byte offset of each
`Lir` index (relative to the function's `code_entry` start). Absolute DWARF offset = `code_base +
FuncCodeRange.code_start + instr_offsets[lir_index]`. `dwarf_funcs_for` maps each `stmt_lines` entry
through this + `span_data.line_at(node)` → a `(code_offset, line)` row.

### 3.5 The `.debug_line` program (already solved)

`DwarfFunc` gains `rows: Vec<(u32 offset, u32 line)>`; `build_line_program` emits a row per entry
(`set_address` → `advance_line` → `copy`), falling back to the single function-entry row when `rows`
is empty (a single-expression body — the current function-granularity behavior, preserved exactly).

---

## 4. Verification — the oracle is a MULTI-ROW dump

The correctness bar (beyond "still compiles / gate stays 0-fail"): a multi-line function must produce
**multiple distinct line rows at ascending offsets**, confirmed by `llvm-dwarfdump --debug-line`. The
first attempt's probe (a 5-line arithmetic body) yielded one row — the regression signal to watch. A
passing result is: rows at lines 3, 4, 5 (say) at increasing addresses, ending in `end_sequence`.
Interactive stepping (does GDB/LLDB actually advance line-by-line) needs `wasmtime -D debug-info`, still
absent on the dev host — so `llvm-dwarfdump` row-count + address-ordering is the landing oracle;
interactive is a manual follow-up (as for D2).

Cross-checks to keep green: the strip round-trip (`strip(debug) == plain`) — the `.debug_line` grows
but stays inert + strippable; the `wasm-encoder` byte-oracle over the executed sections (unaffected —
`stmt_lines` never changes `code`); byte-identity of a `debug = None` build (markers are collected but
only *read* under a debug target).

---

## 5. Scope, cost, and the honest recommendation

**Cost:** the 28-signature `Emit`-swap + comprehensive markers is a wide, mechanical change in
`select.rs` — the hottest, most concurrently-edited file (the effects track lands there repeatedly). A
rebase mid-change is likely; the swap is sed-able so re-applying is cheap, but the marker placements
touch many arms.

**Payoff:** genuine line-by-line stepping in Chrome/GDB/LLDB — the difference between "stops in
`factorial`" and "steps through `factorial`'s lines." High for a debugging story; the guide would get
it for free (it already emits `WasmDebug`).

**Recommendation:** do it as **ONE focused change when `select.rs` is quiet** (not piecemeal on a
cron), landing behind the existing `--target wasm-debug` (no new surface). Sequence:
1. Reinstate the `Emit` wrapper + `peephole_emit` + `instr_offsets` + `DwarfFunc.rows` + the multi-row
   line program (all previously written — recover from the reverted diff or rebuild from §3).
2. Add markers comprehensively (§3.2) — the NEW work vs the failed attempt.
3. Verify with the multi-line dwarfdump oracle (§4); iterate marker placement until a multi-line body
   shows multiple ascending rows.
4. Keep every existing debug test green (function-granularity is the `rows`-empty fallback).

This is a bounded backend project (no IR change, no new artifact, no ABI change) — the earlier "needs
lowering attribution" framing was a misread; the ids are already in `select`, they just need marking.
