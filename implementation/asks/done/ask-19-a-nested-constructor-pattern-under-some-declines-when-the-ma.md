## 19. 🟢 A nested constructor pattern under `Some` declines when the matched list is a parameter — FIXED (seed) — awaiting loop re-probe

> **⏳ PENDING VALIDATION 2026-07-07.** Re-probed against the current seed: the one-step nested pattern
> now COMPILES and runs. All verified:
> - `(def (f xs) (match (List.at xs 0) ((Some (E.Lit n)) n) (None 0)))` on a param list → 5.
> - Nested ctor in a RECURSIVE list walk: `((Some (E.Lit n)) …) ((Some (E.Neg n)) …)` summing/negating
>   over `(list (E.Lit 10) (E.Neg 3))` → 7.
> - Deeper — `(Some (P.Pair (tuple a b)))` on a param list → 7.
> So the ergonomic one-step form (match a list element by its constructor directly, in a recursive walk)
> works now, not just the two-step bind-then-match. **To confirm → done:** pin the one-step form as a
> corpus case (`05-compound-types.sexp`, a `(Some (Ctor …))` on a parameter-list element).

## 19. ⚪ A nested constructor pattern under `Some` declines when the matched list is a parameter

**Finding.** `(match (List.at xs i) ((Some (E.Lit n)) …) (None …))` — a nested constructor pattern
inside the `Some` arm — declines "runtime sum match: unsupported payload binder" when `xs` is a
function parameter. It works when `xs` is an in-place literal list, and works with a two-step bind
(`(Some e)` then inner `match e`). So the boundary is: nested ctor under `Some` + payload element kind
arriving through a parameter (erased to opaque `Heap`).

**Why it touches the seed.** Destructuring a heterogeneously-typed list element in one pattern (matching
a `Node`/`Core` element with its constructor directly) is the ergonomic way to write the reader's/
lowering's list walks. Same family as the sum-match payload-kind-override fixes already landed, extended
one level deeper (through `Option`-of-a-parameter-list-element). Lower priority — the two-step
bind-then-match workaround is clean.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3j), lower priority (has a workaround). Not pinned as a corpus
case yet (the two-step form works, so the idiom is expressible; pin the one-step form when the nested
payload-kind recovery lands). Fix: extend the payload-kind override to a nested constructor pattern
under a sum arm when the list is a parameter. Learning:
`spec/learnings/2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind.md` (noted alongside #18).

**🟢 LOOP-CONFIRMED 2026-07-07 (Run 63).** Re-probed against the running seed: `(match (List.at xs 0) ((Some
(E.Lit n)) n) ((None _) 0))` on a param list of `(E.Lit 5)` emits a VALID component (imports the heap runtime).
Pinned as a fresh gate case *"a constructor pattern nested under Some matches a runtime list element"* → 5 (gate
PASS). Moved pending-validation → done.
