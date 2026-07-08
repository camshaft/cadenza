# Map.insert skips the key and value homogeneity check

*2026-07-08*

**What happened.** Right after the list-growth homogeneity gap was fixed (`List.push`/`List.update`
now reject a differently-typed element), adversarial probing found the *same* gap on the map: the
`Map.insert` operation does not enforce that a map's keys share one type and its values share one
type. `(Map.insert (Map.insert Map.empty 1 10) 2 true)` builds `(map (1 10) (2 true))` — an
Int64-keyed map with one Int64 value and one Bool value — and `(Map.insert (Map.insert Map.empty 1
10) true 20)` builds `(map (1 10) (true 20))` — an Int64 key and a Bool key in one map. Both are
accepted and run. The map *literal* already rejects a mixed-value map (`(map (a 1) (b true))` →
"map values do not share one type"); only the `Map.insert` operation path skips the check.

**Why it is a break.** collections-and-text.md #A Map Associates Keys With Values: "A map MUST
associate keys of one type with values of one type." #A Map Is Built By Functional Construction:
`Map.insert` produces a new map value — a map value, which must satisfy the one-key-type /
one-value-type rule. So inserting a Bool value into an Int64-valued map, or a Bool key into an
Int64-keyed map, must be rejected (CDZ0201), exactly as the mixed-value literal is. Accepting it
builds a map the type system forbids.

**The same shape as the list-growth gap, one type constructor over.** The `(list …)` literal
enforced element homogeneity but `List.push`/`List.update` did not (cycles 15/16); the `(map …)`
literal enforces value homogeneity but `Map.insert` does not — the identical literal-vs-operation
asymmetry, on the map's functional-construction operator. Unlike the list case, the map does not
render-corrupt (each entry prints its actual type), so it is a missing rejection rather than a
wrong value — but it is the same root cause: the homogeneity check lives on the literal path and
was not carried to the operator that builds the same value kind.

**Root cause (likely).** `Map.insert`'s lowering associates the new key/value without checking
their types against the operand map's key type and value type. The fix is the map analogue of the
list-growth fix: `Map.insert` (and `Map.swap`, the value-yielding insert, which has the same
exposure) must check the inserted key's type against the map's key type and the inserted value's
type against the map's value type — CDZ0201 on a mismatch, or a decline if not yet checked.

**The lesson (recurring, now confirmed across two collection types).** Every functional-construction
operator that produces a homogeneous collection must enforce that collection's homogeneity, not just
the literal syntax. The check is a property of the value kind (list element type, map key/value
type), so it must live at every site that produces the value — the literal AND the growth operators.
When the list-growth gap was fixed, the map-growth operator was the next place to look, because it
is the same value-producing-operator-without-the-literal's-check pattern one type constructor over.
A fix should sweep all functional-construction operators of homogeneous collections at once, not one
at a time.

**Corpus cases added.** `spec/semantics/05-compound-types.sexp` §"inserting a value of a different
type into a map is a type error" (`(Map.insert (Map.insert Map.empty 1 10) 2 true)`) and §"inserting
a key of a different type into a map is a type error" (`(Map.insert (Map.insert Map.empty 1 10) true
20)`), both MUST reject CDZ0201, as the functional-construction companions of the map-literal
homogeneity cases. Both carry `(needs maps)` (the `Map.*` ops are realized). Native seed; the
behavior gate catches both (expected reject CDZ0201, observed a running component).
