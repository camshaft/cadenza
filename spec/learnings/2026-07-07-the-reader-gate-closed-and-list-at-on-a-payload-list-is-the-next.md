# The reader gate (built-in Option across a boundary) closed fully — and `List.at` on a payload-bound list is the next accessor

*2026-07-07*

**What happened.** The seed rebuilt and closed the reader gate (backlog item 12 — a built-in
`Option`/`Result` losing its payload kind across a function boundary) across **all** its facets at once:
- `String.from-bytes` through a helper — the reader's symbol-table decode idiom — now compiles and runs
  (`(dec (Bytes.of (list 104 105))) → 2`; ill-formed `0xFF` correctly takes the `None` arm → -1). The
  seed grew a real `gen_runtime_string_from_bytes` (a total, fallible UTF-8 decode on a runtime Bytes),
  and it validates correctly with the *existing* runtime — the in-flight `bytes-is-utf8` WIT op was not
  needed on this path (the runtime's existing String machinery does the check).
- The bare `(Some 42)` through a helper — the *general* built-in-`Option` payload-kind facet, the
  deepest one, untouched through several accessor-specific fixes — now compiles (`(unwrap (Some 42) 99)
  → 42`).

So both corpus cases withheld/pinned-as-todo in earlier cycles — *"a built-in Option is unwrapped by a
helper that binds its payload"* and *"a helper decodes bytes to a string and consumes the fallible
result"* — flipped **todo → PASS** with no edit to their oracles. The accessor-by-accessor closure that
[[2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor]] tracked reached its end: the general
payload-kind fix (not another per-accessor patch) closed the class, exactly as that learning predicted
the real fix would.

The rebuild also surfaced the *next* accessor gap, **Tier 3h**: `List.at` on a list **bound out of a sum
payload** by a `match` arm declines "unsupported dotted-application", while `List.len` on the same
payload-bound list works and `List.at` on a top-level list parameter works. The trigger is precisely
element-access on a payload-bound list. It matters because the natural representation of a
multi-argument call is `KCall (Tuple Int64 (list Core))` — a fn index plus an argument *list* — and
lowering iterates that list with `List.at args i`; but the arg list is a sum-payload field, so
`List.at` on it declining means multi-arg calls can't be lowered by iterating a payload-stored list
(unary calls, `Tuple Int64 Core`, are unaffected).

**Why.** Two connected lessons. First, the reader-gate closure vindicates the accessor-by-accessor
learning's core claim: **per-accessor patching closes symptoms; the class closes only when the built-in
sums get uniform payload-kind recovery.** `Bytes.at` and `List.at` were fixed accessor-by-accessor, but
`String.from-bytes` (a different decline message — it needed its own runtime lowering) and the bare
`(Some 42)` (the general kind-recovery) were the two that remained, and they closed together when the
general fix landed — the symptom-vs-class distinction made concrete. Second, Tier 3h is the *same shape
of gap one level out*: a value bound from a sum payload is an opaque `Heap` handle whose *element-access*
lowering isn't wired for the payload-bound case though its *length* is — the identical "the payload
binder yields the value at a kind/shape the accessor doesn't recognize" pattern that the Option-payload
and runtime-`tuple.N` gaps were. It is the recurring texture of the runtime-value work: each container
(sum payload, tuple, list) must have *every* accessor taught that a payload-bound instance is the same
runtime handle as a top-level one, and the gaps surface one accessor at a time as the compiler reaches
for each. The pattern predicts where to look next: any accessor on any payload-bound container that
hasn't been exercised yet.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"indexing a list bound
from a sum payload yields the element"* (`(K.KK (tuple 7 (list 10 20 30)))` matched, then `List.at xs 0`
→ 10) — pins Tier 3h: element access on a payload-bound list must read like a top-level list, since both
are the same runtime array handle. It records the true oracle (10) and scores **todo** (the seed
declines cleanly today), turning green when `List.at`'s payload-bound lowering aligns with the top-level
case — which unblocks the natural multi-argument-call representation. **Backlog item 12 is now resolved**
(all facets green, both withheld cases passing), and **a new item 17** records Tier 3h (the payload-bound
`List.at` gap and its multi-arg-call consequence). The list-patterns item 13 remains the ergonomic
alternative — if list patterns land, a compiler destructures an arg list by pattern rather than
`List.at`-iterating it, so 3h and 13 are two routes to the same multi-arg-call capability.
