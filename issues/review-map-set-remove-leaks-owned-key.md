# Map.remove / Set.remove LEAK an owned-temporary key (twin of the just-fixed lookup/contains gate)

**Severity:** correctness/soundness — a heap LEAK (not a wrong value). Valid wasm, `cdz check` clean,
corpus-green (the returned map/set is correct), but `live-objects > 0` after the run.

**Where:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs`
- `Core::MapRemove` emit — **select.rs:5398–5410**
- `Core::SetRemove` emit — **select.rs:5456–5468**

## The bug

The runtime ops BORROW their key/elem and DO NOT free it:
- `op_map_remove` (`cdz-runtime/src/lib.rs:6721`) — docstring at **lib.rs:6716–6719**: *"CONSUMES `m`
  (moves through to the result), BORROWS `key`."* `champ_remove_fbip` reads the incoming `key` only via
  `champ_hash`/`champ_eq`; the only `op_drop`s it issues are on the map's OWN stored entry columns
  (`handles[base + t]`, lib.rs:6633/6657), never on the passed-in `key`.
- `op_set_remove` (`lib.rs:7217`) / `set_remove_h` (`lib.rs:7223`) — docstring at **lib.rs:7213–7215**:
  *"CONSUMES `s`, BORROWS `elem`."* Same story.

But the EMITS drop the key **nowhere** — and worse, their comments assert the opposite:
- select.rs:5395–5397: *"consumes the map, borrows the key — the boxed key is an owned temporary
  dropped inside the op."* ← FALSE. The op does not drop it.
- select.rs:5453–5455: *"borrows the element — the boxed element is dropped inside the op."* ← FALSE.

So an OWNED-TEMPORARY key/elem handle that materializes at the emit — a boxed large Int
(`op_box_int` heap-allocs when `!fixnum_fits`, lib.rs:850), a **constant String** leaf (`ConstStr` →
`bytes-alloc`), or a **compacted rope** (`key_needs_compaction` fires → fresh flat leaf) — is neither
consumed by the op nor dropped by the frame. It LEAKS on every `Map.remove` / `Set.remove` call.

This is the exact twin of the lookup/contains ownership hazard that `05e37221`
(`key_handle_is_owned_temporary`) just fixed — but in the OPPOSITE direction. There, the op borrowed
and the emit **over-dropped** a borrowed key (use-after-free of a live owner). Here, the op borrows and
the emit **under-drops** an owned temporary (leak). The remove emits were never brought to parity: they
predate the gate and assume a consume that the runtime does not perform. `MapLookup`/`SetContains` got
the `key_handle_is_owned_temporary` drop-when-owned treatment; `MapRemove`/`SetRemove` did not.

Note the asymmetry is real vs the INSERT ops: `op_map_insert`/`op_set_insert` genuinely CONSUME the key
(it is stored into the node), so their emits correctly drop nothing. The remove/lookup/contains ops
BORROW. The gate landed for two of the three borrow ops; remove is the miss.

## Fix (mirror the landed gate)

Both remove emits should `tee` the key/elem into a scratch slot and, guarded on
`key_handle_is_owned_temporary(db, key, &key_ty)?`, `drop` it after the borrowing `map-remove`/
`set-remove` — identical to the `MapLookup` (select.rs:5561–5569) and `SetContains`
(select.rs:5500–5507) shape now in the tree. A BORROWED key (param / kept-local / live sum-payload
projection) is left to its owner exactly as there; an owned box/const/compacted-rope key is dropped.
Also correct the two false "dropped inside the op" comments.

## Reproducer (leak — needs the debug-counters oracle, NOT the plain gate)

Because the returned value is CORRECT, the standard corpus gate cannot catch this — only the
`live-objects` leak oracle (`#[ignore]` debug-counters tests in `rcdzc/src/tests.rs`, e.g.
`runtime_value_eq_leaves_no_live_objects` at tests.rs:2604) will. Add a balance test in that family:

    (module m
      ; a LARGE-int key forces op_box_int to heap-allocate (fixnum_fits is false), so the key is an
      ; owned temporary; Map.remove borrows it and the emit currently never drops it → it leaks.
      (def (main)
        (let ((m (Map.insert (map) 100000000000 1)))
          (Map.size (Map.remove m 100000000000))))   ; returns 0, but leaks the boxed key
      (export main))

Assert `rt.live_objects() == 0` after the call (it is currently nonzero). A constant-String key
(`(Map.remove m "abc")` on a `Map String Int64`) and a `Set.remove` with either shape are the sibling
witnesses. All three should net to 0 live cells once the emit drops the owned key.

## Verified

Static: confirmed the runtime borrows (docstrings + `champ_remove_fbip` drops only stored columns), the
emit issues no key drop, and `op_box_int`/`ConstStr`/`key_needs_compaction` all produce an owned heap
handle at the emit. Consistent with the ownership model the sibling `05e37221` fix codified.
