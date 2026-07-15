# Map equality miscompiles across the const/runtime construction boundary

*2026-07-08*

**What happened.** Two structurally-equal maps compare UNEQUAL when one was built with a compile-time
constant key and the other with a run-time-computed key. `(let ((j (+ 2 3))) (map (j 1)))` and `(let ((k
5)) (map (k 1)))` are the same map `{5:1}` — `(+ 2 3)` is 5, both render `(map (5 1))`, both `Map.lookup
5` → `(Some 1)`, both `Map.len` → 1 — yet `(= (map (j 1)) (map (k 1)))` returns `false`. A const-key
literal compares equal to a const-key literal and to a `Map.insert` map (all true), but a computed-key
(runtime-constructed) map compares false against a const one. This is a wrong VALUE, not a missing
rejection.

**Why it is a break.** core-semantics.md #Equality Is Structural: two values are equal exactly when their
canonical forms coincide. The two maps have identical canonical forms (`(map (5 1))`), so they MUST be
equal. Returning false is a structural-equality miscompile — the compiler compares the two maps'
different INTERNAL representations (a const-folded map value vs a runtime heap-map handle) rather than
their values.

**Why it is WORSE than the list/tuple case.** For lists and tuples, the analogous runtime-compound
equality honestly DECLINES: `(let ((x (+ 2 3))) (= (list x) (list 5)))` and the tuple form both decline
"runtime compound equality (heap walk) not yet emitted" — a safe reject-don't-miscompile outcome. The map
equality path, by contrast, is realized enough to EMIT an equality but it silently answers false for a
runtime-constructed map. So maps miscompile where lists/tuples safely decline.

**Root cause (likely) — the map equality path compares representations, not values, across the
const/runtime boundary.** A `(map …)` literal with all-constant keys/values is const-folded to a map
VALUE; a literal with a computed key is built as a runtime heap map (the persistent-map handle). The
equality operator, given a const-folded map on one side and a runtime heap map on the other, compares
them by their representation (fold vs handle) instead of walking both to their canonical entries — so two
maps with identical entries but different construction representations compare unequal. The fix is to
compare maps by their canonical entry set (the value), independent of representation — or, if the
runtime-vs-const map equality is not yet implemented, to DECLINE it as list/tuple equality does, never to
answer false.

**The lesson (the recurring family + the const↔runtime dimension).** A value-comparison must be by VALUE
across every construction path — the const/runtime boundary must be invisible to equality (the same
discipline as the const-fold↔runtime agreement checks for arithmetic). Here the map path violates it and,
unlike list/tuple, does so as a miscompile rather than a decline. The tell: `{5:1}` == `{5:1}` is true
when both keys are constants but false when one key is computed — equality branched on how the map was
built, which structural equality forbids.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a map with a computed key equals the same
map with a constant key" — `(let ((j (+ 2 3))) (let ((k 5)) (= (map (j 1)) (map (k 1)))))` MUST be true.
Gated `(needs collections)`, realized; the behavior gate catches it (expected true, observed false). A
generation whose map equality cannot yet compare a runtime-constructed map against a const one declines
rather than answering false.
