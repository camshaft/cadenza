## 76. ✅ FIXED 2026-07-08 — List.concat wired (runtime vec-concat surfaced); the ideal list<Lir> codegen is unblocked and checked +/- now emit end-to-end.

**Status: BLOCKING the ideal Phase-1 codegen shape. This is a WIRING task, not a spec or runtime gap — the
runtime op already exists; the seed compiler just never surfaced it.** `List.concat` declines "unsupported
dotted-application" on the seed, but the runtime primitive is done: `op_vec_concat` (RRB relaxed-radix,
`crates/cdz-runtime/src/lib.rs:1579`) is implemented, exported as `vec-concat` in the WIT
(`lib.rs:1944`), and oracle-tested (`vec_concat_matches_oracle`, `vec_concat_empty_operand_identity`) — but
it is flagged **`never used` (dead_code)** because the seed's compiler dispatch never maps `List.concat` →
`vec-concat`.

**Reproducers (stable seed, `emit`):**
```
(module m (def (main) (List.len (List.concat (list 1 2) (list 3 4)))))          ; → decline (want 4)
(module m (def (f a b) (List.len (List.concat a b))) (def (main) (f (list 1 2) (list 3 4))))  ; → decline (want 4)
; CONTRAST — the analogous BYTES op is fully wired: (Bytes.concat …) → works.
```

**The wiring (mirror `List.push` → `vec-push`).** In `codegen.rs` the emit dispatch already has, at
~L7781-7784:
```
(Some("List"), Some("push"))   => return self.gen_runtime_list_push(elems, env, ctx),   // emits call himport::VEC_PUSH
(Some("List"), Some("update")) => return self.gen_runtime_list_update(...),
(Some("List"), Some("len"))    => return self.gen_runtime_list_len(...),
(Some("List"), Some("at"))     => return self.gen_runtime_list_at(...),
```
`List.concat` needs the parallel arm — `(Some("List"), Some("concat")) => gen_runtime_list_concat(...)` —
whose `gen_runtime_list_concat` emits its two list operands (both `Kind::Heap`, each `dup`'d per the ask-63
consume-contract since `vec-concat` CONSUMES both) then `call himport::VEC_CONCAT`. Likely also: add
`VEC_CONCAT` to the `himport` index set (the generated `wit_envelope`/`himport` import list — the WIT export
exists, so the index just needs to appear in the compiler's import order), and a `shape_of` arm so a
`List.concat` result carries its element shape (mirror the `List.push` shape arm at ~L905-909: a concat's
element shape is the operands' element shape). Result kind is `Kind::Heap` (a list handle), like `vec-push`.

**Semantics (matches the runtime + `Bytes.concat`).** `(List.concat a b)` = a new immutable list whose
elements are `a`'s followed by `b`'s (the runtime's `op_vec_concat` contract); empty-operand identity
(`vec_concat_empty_operand_identity` already tests it). This is the list companion of the specified
`Bytes.concat` and the append rule (collections-and-text.md §"A List Is Immutable Under Growth"). If the
spec should also *name* `List.concat` (the capability currently names only append + update), that is a small
spec fold — but the operation is already realized in the runtime and tested, so this ask is the compiler
wiring; the spec fold can follow.

**Why it's load-bearing (and why NOT worked around).** `compiler-pipeline.md` §Representation mandates a
typed instruction SUM serialized by an exhaustive match — so a cdzc function body is a `list Lir` assembled
by concatenating an operand's instruction list ++ the operator's sequence (the `++` is `List.concat`).
Phase-1 checked arithmetic is ready otherwise: `serialize : Lir → bytes` (exhaustive) and the
overflow-guarded op sequence are designed+validated (runs 1+2→3, -5+-3→-8; traps at Int64.max+1). Only the
`list Lir` assembly is blocked. The old `compiler.cdz` worked around the absence with a push-based
`code-cat` (O(n²)) or raw `Bytes` concatenation — the very "byte emission / pseudo-structure" the pipeline
spec steers away from and the rewrite exists to replace — so it is NOT re-taken here; wire `vec-concat`
instead.

**Priority.** 🔴 HIGH for the rewrite's backend shape; small runtime cost (op already implemented + tested).
Related: ask-62 (list migration — flagged "no List.concat ⇒ code-cat O(n²) WATCH"), `Bytes.concat` (the
wired analogue to mirror), `gen_runtime_list_push` (the exact emit pattern).

**Acceptance signal.** `(List.concat (list 1 2) (list 3 4))` → a length-4 list `1 2 3 4` (const and
runtime); the runtime `op_vec_concat` dead_code warning clears; cdzc's `select`/`serialize-body` assemble +
emit a `list Lir` body.
