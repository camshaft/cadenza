## 17. 🟢 `List.at` on a list bound from a sum payload declines (blocks the natural multi-arg-call rep) — FIXED 2026-07-07

**Resolution (2026-07-07, seed side).** Root cause was NOT payload-specific: there was simply NO runtime
`List.at` emitter at all — `gen_dotted_apply` had runtime `List.push`/`update`/`len` but not `at`, so
any non-const-folding `List.at` fell to "unsupported dotted-application". (A top-level `List.at (list …)
i` "worked" only by const-folding the literal list; a payload-bound list is a genuine Heap handle that
can't fold.) Added `gen_runtime_list_at` (mirrors `gen_runtime_bytes_at` over `vec-len`/`vec-get`): a
FALLIBLE index → `(Some elem)` / `(None unit)`. A list element is ALREADY a boxed handle (stored via
`box_scalar` at construction), so `vec-get` returns it directly as the `Some` payload — the caller's
match unboxes it via the payload-kind override. Also wired `List.at` in `infer_list` (list→Heap,
index→Int64, result→Heap Option) and `shape_of_list` (→ `Option<element-shape>` for rendering). The
multi-arg-call idiom `KCall (Tuple Int64 (List Core))` lowered by `List.at args i` now works — verified
by an `ev` that recurses over a payload arg list summing `KConst`s → 42. Pinned by corpus *"indexing a
list bound from a sum payload yields the element"* (→ 10) and *"a multi-argument call node is evaluated
by iterating its payload arg list"* (→ 42). Gate 527/0, component-check 532/0, ignition byte-identical.
See [[runtime-list-at-fallible-index]].

**Original finding (for history).**

**Finding.** `List.at` on a `list` **bound out of a sum-type payload** by a `match` arm declines
"unsupported dotted-application", for any element type. `List.len` on the same payload-bound list
**works**, and `List.at` on a **top-level** list parameter **works** — so the gap is specifically
element-access on a payload-bound list. Same shape as the earlier payload-kind gaps (Option payload,
runtime `tuple.N`): a value bound from a sum payload is an opaque `Heap` handle whose *element-access*
lowering isn't wired for the payload-bound case though its *length* is.

**Why it touches the seed.** The natural representation of a multi-argument call is `KCall (Tuple Int64
(list Core))` — a fn index plus an argument *list* — and lowering iterates that list with `List.at args
i` per argument. But the arg list is a sum-payload field, so `List.at` on it declining means multi-arg
calls can't be lowered by iterating a payload-stored list (unary calls, `Tuple Int64 Core`, work).
Not a spec gap — the language plainly permits it (top-level `List.at` works); it is seed lowering.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3h). Pinned by `05-compound-types.sexp` *"indexing a list bound
from a sum payload yields the element"* (`(K.KK (tuple 7 (list 10 20 30)))` matched, `List.at xs 0` →
10), scores **todo** (declines cleanly today). Fix: make `List.at` on a payload-bound list lower like a
top-level list (both are the same runtime array handle). Note the overlap with item 13 (list patterns):
if list patterns land, a compiler destructures an arg list by pattern rather than `List.at`-iterating
it, so 17 and 13 are two routes to the same multi-arg-call capability. Learning:
`spec/learnings/2026-07-07-the-reader-gate-closed-and-list-at-on-a-payload-list-is-the-next.md`.

---
