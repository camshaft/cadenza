# adv: wasm Set.to-list FALSE-DECLINES compound (tuple/list) elements — rust computes; a backend divergence

**breaker found 2026-07-20 (trunk a80ae8a41-era).** `Set.to-list` over a Set whose ELEMENTS are compound
(tuple or list) DIVERGES between backends: **wasm rejects at compile time** ("Set.to-list element shape has
no orderable descriptor") while **rust computes the canonical (sorted) order**. Since Set.to-list is defined
to sort by canonical element-VALUE order (19-sets:677/703), and the compound element types ARE orderable
(wasm's own bare `<` computes on them — see below), the wasm decline is a FALSE-DECLINE: wasm's Set.to-list
descriptor path lacks the orderable descriptor for compound elements that (a) wasm's OWN `<` operator has and
(b) rust's Set.to-list has.

## Severity
Backend divergence / wasm false-decline. Not a wrong-VALUE miscompile (wasm rejects rather than mis-orders),
but the two backends disagree on whether the operation is even VALID — a program using Set.to-list over a
tuple/list-element set compiles+runs on rust but is rejected on wasm. Blocks a legitimate operation on wasm.

## The divergence (minimal)
```
(do (def (main)
      (match (List.at (Set.to-list (Set.of (list (tuple 3 1) (tuple 1 2) (tuple 2 0)))) 0)
        ((Some p) (match p ((tuple a b) (+ (* 10 a) b)))) ((None _u) -1)))
    (export main))
```
- **wasm**: `cdz compile` → `error: Set.to-list element shape has no orderable descriptor` (REJECTED).
- **rust** (`cdz run-rust`): `value 12` — computes; the 3 tuples enumerate in canonical lexicographic order
  (1,2)(2,0)(3,1) → first-components-weighted `(tuple 12 20 31)`.
- Same divergence for LIST elements: `(Set.to-list (Set.of (list (list 3 1) (list 1 2))))` — wasm declines,
  rust computes (len 2).

## Why it's a FALSE-decline (the tell)
Compound element types ARE orderable — wasm's own bare `<` computes on them:
- `(< (tuple 1 2) (tuple 1 3))` → wasm **1** (tuples orderable on wasm).
- `(< (list 1 2) (list 1 3))` → wasm **1** (lists orderable, blessed lexicographic — 03-equality:302).
So wasm HAS a total order for tuple/list values in its `<` path, but wasm's Set.to-list SORT path does not
reuse it ("no orderable descriptor") — while rust's Set.to-list derives Ord and sorts fine. Control: a
String-element Set.to-list (blessed order) COMPUTES on wasm (len 3) — so wasm accepts blessed-scalar elements
and only false-declines the COMPOUND ones.

## Fix direction (owner: v-runtime — Set.to-list emit / the orderable-descriptor layer for wasm)
wasm's Set.to-list must build the orderable descriptor for a COMPOUND element type by composing its
components' orders (the same value-cmp lexicographic walk that wasm's compound `<` / `Core::ValueCmp` already
uses), instead of only recognizing scalar/String/Symbol leaf descriptors. Then wasm computes the canonical
tuple/list order like rust does. Alternatively, if the ruling is that Set.to-list is NOT defined for
compound elements, BOTH backends must decline (rust's compute would then be the bug) — but that contradicts
Set.to-list's "canonical element-value order" definition + the fact that the elements are orderable.

## Probes (all at trunk a80ae8a41-era)
- tuple-element: wasm declines / rust `(tuple 12 20 31)`.
- list-element: wasm declines / rust len 2.
- bare `(< tuple tuple)` / `(< list list)`: wasm computes (1) — proves orderability.
- String-element control: wasm computes (len 3).

Not breaker's lane to fix. Filed adv + issue to v-runtime (Set.to-list wasm emit / orderable-descriptor).

## Map.to-list twin — behaviorally CONFIRMED (breaker 2026-07-20 ~11:34)
The identical divergence exists for `Map.to-list` over a COMPOUND-KEY map (v-runtime's reply flagged it +
their memory pins the `Ty::Map(key,val)` KEY arm ~13013 as the same bug). Verified:
`(Map.to-list (Map.insert (Map.insert Map.empty (tuple 3 1) 100) (tuple 1 2) 200))` — wasm: compile error
"Map.to-list key/value shape has no orderable descriptor"; rust: computes (len 2). Same scalar-only
orderable-descriptor guard, Map side. v-runtime is fixing BOTH (Set + Map to-list) in the one BRICK-2-batched
change (reuse value_cmp_shaped). No separate adv — same root cause, same fix.

---
## Triage (corpus-bugfix, 2026-07-20, trunk af0a646f7)
RESOLVED + PINNED. v-runtime's compound-element orderable-descriptor fix landed. Verified on wasm:
- tuple-element Set.to-list: (Set.to-list (Set.of (list (tuple 3 1)(tuple 1 2)(tuple 2 0)))) -> canonical
  order (1,2)(2,0)(3,1); first element -> 10*1+2 = 12. (was 'element shape has no orderable descriptor'.)
- list-element Set.to-list -> len 2; Map.to-list compound-KEY twin -> len 2 (both compute).
Existing pin "Set.to-list orders a set of COMPOUND (tuple) elements lexicographically" (19-sets:803) PASSES
on trunk. Corpus side complete. Marked RESOLVED.
