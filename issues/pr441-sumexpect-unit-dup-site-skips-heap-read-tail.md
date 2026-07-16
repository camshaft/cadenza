# PR review comments — mirrored from GitHub PR #441 (Copilot inline)

- **PR:** #441 "fleet: sixty-first batch (…, SumExpect retain, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs` (get_op Unit @1171, SumExpect dup-site @7487)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592691424, 3592691434
- **Links:** https://github.com/camshaft/cadenza/pull/441#discussion_r3592691424 , #discussion_r3592691434

## Comments (verbatim)
> `get_op` returns `Ok(None)` for `Ty::Unit`, so `scalar_leaf` is currently false for Unit. That means the new `SumExpect` retain logic can mark a Unit-typed expect as a dup site, but the emit path for dup sites skips `emit_heap_read_tail` (which is where Unit's sentinel handle gets dropped). This can produce invalid wasm (extra handle left on the stack for a Unit-typed block).
> In the `SumExpect` emit, the dup-site fast-path skips `emit_heap_read_tail`. That is correct for compound handles, but wrong for `Ty::Unit` because `get_op` is None for Unit too and the heap-read tail is responsible for dropping the inline-unit sentinel. Guard the dup-site branch so Unit still goes through `emit_heap_read_tail` (drop).

## Liaison triage — CONFIRMED against trunk — Unit-in-heap wasm soundness (pr402/pr388 class)
Confirmed: `get_op` → `Ok(None)` for `Ty::Unit`, so a Unit-typed `SumExpect` can be marked a dup-site,
and the dup-site fast-path SKIPS `emit_heap_read_tail` (select.rs:7486, "[scalar | handle | nothing]")
— which is exactly where Unit's inline sentinel (IMM_UNIT) would be dropped. Result: an extra handle
left on the operand stack for a Unit-typed block → invalid wasm. This is the same Unit-across-heap class
as pr402 (`box_op`/`get_op` Ok(None) conflating Unit) and pr388 (closure Unit result). FIX: guard the
SumExpect dup-site branch so a `Ty::Unit` still goes through `emit_heap_read_tail` (the drop). Route to
`corpus-bugfix` PM (wasm-backend Unit soundness) to repro (a SumExpect extracting a Unit payload
consumed while the sum is live) + fix. Fix on `trunk`. Quotes + links in queue file.
