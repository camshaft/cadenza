;; GAP (2026-07-14, seed rcdzc — MISSING ops): a `Map` and a `Set` can be BUILT and QUERIED but NOT
;; ENUMERATED. There is no `keys` / `values` / `entries` / `to-list` / `fold` on `Map`, and no
;; `to-list` / `elements` / `fold` on `Set`. A program can `insert`/`lookup`/`size` a map and
;; `of`/`contains`/`union`/`len` a set, but it CANNOT visit their contents.
;;
;;   `(. Map keys)`     → CDZ0201 "the `Map` module has no member `keys`"   (same for values/entries/to-list/fold/iter)
;;   `(. Set to-list)`  → CDZ0201 "the `Set` module has no member `to-list`" (same for elements/fold)
;;
;; The FULL realized surfaces (prelude.rs `map_module`/`set_module`):
;;   Map: empty, insert, lookup, remove, size, swap, take        — NO enumeration
;;   Set: of, contains, insert, remove, len, union, intersection, difference — NO enumeration
;; `Set.of(list)` builds a set FROM a list, but there is no `Set.to-list` inverse.
;;
;; WHY THIS IS A REAL GAP (a compiler cannot avoid it): a symbol table (`Map String Ty`) must be WALKED
;; to emit every binding / report every definition; a set of free variables or reachable defs must be
;; ENUMERATED to render or serialize it. Lookup-by-key is not enough — you cannot enumerate the keys to
;; look up. Every real pass that PRODUCES a collection and then must OUTPUT its contents is blocked.
;;
;; SPEC NOTE: `collections-and-text.md` §"Map Iteration Is Deterministic" states "Iterating a map MUST
;; visit its entries in a deterministic order derived from the keys" and §"A Map Renders As Its Entries
;; In Canonical Key Order" — so iteration is a described capability AND the canonical form already
;; renders the entries (a `(map ("a" 1) ("b" 2))` value prints its pairs). But no PROGRAM operation
;; exposes that iteration: the entries are observable in the RENDERED value, not to the running program.
;;
;; ASK: a `Map.to-list : (Map k v) → (List (Tuple k v))` (entries in canonical key order) and a
;; `Set.to-list : (Set a) → (List a)` (elements in canonical order) — the enumeration inverses of
;; `Map.insert*` / `Set.of`. `fold`/`keys`/`values` can build on `to-list`. This file's `main` wants the
;; number of keys whose value is > 0, which needs to VISIT the entries — impossible today.
;;
;; 🔑 TRACTABILITY (the runtime capability ALREADY EXISTS): `cdz-runtime/wit/runtime.wit` declares
;; NON-ALLOCATING CURSOR iteration ops used internally for rendering / equality / folds —
;;   `map-iter`/`map-iter-next`/`map-iter-key`/`map-iter-val` (indices 42–45) and
;;   `set-iter`/`set-iter-next`/`set-iter-elem` (51–53).
;; So the runtime already walks a Map/Set in canonical key order (that IS how the canonical form renders
;; the entries). The missing piece is purely FRONT-END: a `to-list` prelude field + its scheme + a
;; `lower.rs`/backend emit that runs the cursor loop building a `(List (Tuple k v))` / `(List a)`. No new
;; runtime op is needed — a dedicated increment wiring the existing cursor ops to a surface operation.
(do
  (def (count-positive (: m (Map String Int64)))
    ;; INTENDED: fold the map's entries, counting values > 0. Cannot be written — no Map enumeration.
    ;; The closest available op is `Map.size`, which counts ALL keys, not those matching a predicate.
    ((. Map size) m))
  (def (main) (count-positive (Map.insert (Map.insert (map) "a" 5) "b" (- 0 3))))
  (export main))

;; BLOCKED 2026-07-15: NOT a front-end fix — needs a RUNTIME op (canonical value-ordering for to-list; CHAMP cursor walks HASH order, no front-end sort). Feature gap, no miscompile. Routed the runtime half to v-runtime; fix-map-set-enumeration escalated ownership to concierge + closed. Do NOT reassign as a plain fix agent until the runtime op exists.
