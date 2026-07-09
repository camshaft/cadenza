## 61. 🟡 DESIGN (operator direction) — Map pattern matching is a SEPARATE PHASE, key-directed lookup patterns (gated `(needs map-patterns)`)

**Operator direction (2026-07-07, verbatim intent).** *"I wonder if we should also think about map pattern
matching while we're here … we can have this as a different phase too — so the corpus can have map-pattern
needs."* So: capture the design + gate corpus cases behind `(needs map-patterns)`, but keep it a SEPARATE phase
from the `Map.*` OPERATION surface (empty/insert/swap/lookup/remove/take/size, ask-81/82) — the ops land first;
patterns are their own spec-first item.

**Why a map pattern is a DIFFERENT mechanism from the structural patterns.** The existing patterns —
sum-constructor `(Ctor binder)`, tuple `(tuple a b)`, list-element `(list x .. rest)` (ask-13), literal,
wildcard — all deconstruct a value whose SHAPE IS STATICALLY KNOWN FROM ITS TYPE (a tuple's arity, a sum's
variant set, a list's element structure). A MAP is fundamentally different: its key set is a RUNTIME collection,
NOT a static part of its type (`collections-and-text.md §A Map Associates Keys With Values`, and the reason
`(= map record)` is a type error — a map's key set is not a shape). So a map pattern cannot "destructure the
fixed shape"; it must be a KEY-DIRECTED LOOKUP pattern — a QUERY, not a structural decomposition:

    (match m
      ((map (k1 p1) (k2 p2) .. rest)  …)   ; m HAS key k1 bound to a value matching p1, k2→p2,
                                            ;   and `rest` = the map with k1,k2 removed
      ((map)                          …))   ; m is empty

Each `(k p)` entry is a lookup: `k` is an expression evaluated to a key VALUE (compared by value, §Keys Are
Compared By Value), and `p` is a pattern the associated value must match; the arm matches only if EVERY named
key is present AND its value matches. A trailing `.. rest` binds the remaining map (the operand minus the named
keys), and no rest binder means the map must have EXACTLY the named keys. This is the map analogue of the
list element-with-rest pattern (ask-13), but keyed by value-lookup rather than position.

**Exhaustiveness is not shape-driven (open question for the spec).** A map has unboundedly many possible key
sets, so — unlike a sum (finite variants) or a tuple (fixed arity) — a set of map-key arms is NOT exhaustive by
covering "all shapes." Exhaustiveness must come from a catch-all / bare-name / `.. rest`-with-no-required-keys
arm (the "any map" case), the same way a list match needs the empty + rest arms. The spec clause must state this
(a map match with only specific-key arms and no catch-all is non-exhaustive → CDZ0210).

**Today's idiom (why this is sugar, not a capability gap).** `Map.lookup : m k -> (Option v)` already gives
"match on whether a key is present and bind its value" via an ORDINARY sum-match on the returned Option:
`(match (Map.lookup m k) ((Some v) …) ((None _) …))`. So map patterns are ERGONOMIC sugar over lookup-then-
Option-match, not a new capability — which is exactly why they can be a later phase without blocking the map
operation surface.

**Proposed plan (spec-first, phased).**
1. (later cycle) `core-semantics.md §Pattern Matching` gains a normative clause *"A Map Is Matched By
   Key-Directed Patterns"*: the `(map (k p)… .. rest)` form, its lookup semantics (all named keys present + values
   match; `rest` = operand minus named keys), value-key comparison, and the exhaustiveness rule (catch-all/rest
   required; specific-key-only ⇒ CDZ0210).
2. Corpus cases gated `(needs map-patterns)` — pin the semantics as gate pressure (present-key match, absent-key
   non-match falls through, `.. rest` binds the remainder, exhaustiveness rejection). These land NOW (gated,
   skip) so the corpus carries the intent; the clause + lowering follow.
3. Lowering (a later cycle, after the `Map.*` ops land): desugar a map pattern to `Map.lookup` per named key +
   `Map.remove` for the rest binder (all over the frozen CHAMP ops) — a map pattern is literally lookup-then-
   match-then-remaining, so it rides the operation surface with no new runtime op.

**Status.** 🟡 DESIGN ask, operator-directed, SEPARATE PHASE from the map ops (ask-81/82). Corpus cases gated
`(needs map-patterns)` land now (skip until spec+lowering). Related: ask-81/82 (the Map operation surface it
rides on — `Map.lookup`/`Map.remove` are its lowering primitives), ask-13 (list element+rest patterns — the
positional analogue), `map-operation-surface-spec` / `patterns-compose-spec-must` (memory).