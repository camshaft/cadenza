# Payload-bound `List.at` fixed — multi-argument calls are now representable, and the payload-accessor pattern is complete for lists

*2026-07-07*

**What happened.** The seed rebuilt and fixed Tier 3h (backlog item 17): `List.at` on a list **bound
out of a sum payload** by a `match` arm now reads its element — `(K.KK (tuple 7 (list 10 20 30)))`
matched, `List.at xs 0` → 10. The corpus case pinned as *todo* last cycle flipped **todo → PASS**. This
unblocks the capability it gated: the natural **N-ary call representation**. A call node `KCall (Tuple
Int64 (List Core))` — a function index plus an argument *list* of sub-nodes — can now be evaluated (or
lowered) by iterating its payload-bound arg list: `List.len xs` for the count, `List.at xs i` for each
argument node, each consumed recursively. Verified end-to-end: a `KCall` carrying three `KConst` args
[10 20 12], summed by walking the payload list → 42. Before the fix, multi-arg calls had to be a custom
cons-sum of args (the cons-list workaround); now the built-in `list` carries them.

**⚠️ Root cause was NOT payload-specific — I mis-framed it, and the miss is instructive.** The seed-side
fix note (SEED-GAPS Tier 3h / [[runtime-list-at-fallible-index]]) shows the real cause: there was
**no runtime `List.at` emitter at all** — `gen_dotted_apply` had runtime `List.push`/`update`/`len` but
not `at`, so *any* non-const-folding `List.at` fell to "unsupported dotted-application". A top-level
`List.at (list …) i` only "worked" because the literal list **const-folds**; a payload-bound list is a
genuine Heap handle that cannot fold, which is why the gap surfaced there first. The fix added
`gen_runtime_list_at` (a fallible index over `vec-len`/`vec-get`, mirroring `gen_runtime_bytes_at`),
plus `infer_list` and `shape_of_list` wiring. So it was a *missing accessor*, not a *payload-shape
mismatch* — my "payload-bound X behaves like top-level X" framing named the symptom's location, not its
cause.

**Why.** The primary lesson is the one my mis-framing surfaced: **const-folding masks a missing runtime
accessor, and a payload-bound value is the first place the mask comes off.** `List.at` had no runtime
emitter, yet `List.at (list 1 2 3) i` passed for cycles — because the literal list const-folds, so the
access never reached the (absent) runtime path. The gap became visible only when the list was a
payload-bound Heap handle that *cannot* fold. This is the *same trap* as the "scale limit" misdiagnosis
([[2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis]]): reasoning from a clean
analogue (a top-level literal list) that happens to const-fold hides the real cause (no runtime lowering
at all), and points the diagnosis at the wrong axis (I said "payload-shape mismatch"; it was "missing
accessor"). The corrected rule, now proven twice: **when a construct works at the entrypoint / on a
literal but fails through a boundary or on a runtime value, suspect the entrypoint case is const-folding
past a gap the runtime path never had — check whether the runtime emitter exists at all, before
theorizing about shapes or scale.** A const-foldable positive control is not evidence the runtime path
works.

There *is* a real secondary pattern — a compiler's IR nodes are sums whose payloads are compounds
(tuples, lists, sub-nodes) reached by every accessor, so each accessor must work on a payload-bound
(non-foldable, genuinely runtime) value — but it is a statement about *where gaps surface* (the
non-folding payload-bound position is the honest test), not about a distinct payload-specific defect
class. With `List.at` added, the built-in containers a compiler stores in its nodes are reachable
through a payload binder by the accessors the compiler uses (tuple `.N`, `List.len`/`at`, sum `match`);
a future accessor with no runtime emitter (a payload-bound `Map.get`, say) would be the same shape
again — and the first sign would again be a const-folding entrypoint case passing while the runtime
case declines.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"a multi-argument call
node is evaluated by iterating its payload arg list"* — pins the capability item 17 unblocked: a `KCall
(Tuple Int64 (List Core))` whose arg list is bound from the payload, iterated by `List.len` +
`List.at`, each argument a recursive sum value evaluated in turn, `KCall(9, [10 20 12])` → 42. It is
deliberately distinct from the single-element payload-`List.at` case (which pins the accessor in
isolation): this walks the *full* payload list of *heap sub-nodes* and consumes each, the exact shape a
`lower`/`ev` pass over a variable-arity call node takes. It **PASSES**. **Backlog item 17 is resolved**;
the multi-arg-call representation is now available to the compiler without a cons-sum workaround. The
overlap with item 13 (list patterns) stands: 17 gives the `List.at`-iteration route to N-ary calls;
13 (if it lands) would give the pattern-destructuring route — the `List.at` route works today, so 13
becomes purely an ergonomic improvement, not a capability gate.
