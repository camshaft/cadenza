# PR #1948 review — spec/semantics/07-type-system.sexp (breaker) — MERGED — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1948 — MERGED 2026-08-04T04:36:28Z (3-pin CDZ0216 compound-key
descent). Copilot (id 3709467343) flags the doc for the list-set-element case understates the list
nesting. Corpus zone (`.sexp`) — breaker owns.

## doc says the Set element type is `(List (-> Int64 Int64))` but the input is a list-of-list-of-closures, so it's `List (List (-> Int64 Int64))` (Copilot, 07-type-system.sexp:1971) — doc-accuracy [VERIFIED, LOW]
> The doc for this case describes the Set element type as `(List (-> Int64 Int64))`, but the input is a
> *list of list of closures* (`(list (list (fn ...)))`), so the Set element type is `List (List (->
> Int64 Int64))`. As written, the explanation is a bit misleading about which level the descent is
> happening at.

VERIFIED on trunk. The case "a LIST of closures as a set element is rejected" has input
`(Set.len (Set.of (list (list (fn ((: x Int64)) (+ x n))))))` — `Set.of` over `(list (list (fn …)))`, so
the Set's element type is `List (List (-> Int64 Int64))` (TWO list levels). The doc says "descend through
the collection's ELEMENT type (a `(List (-> Int64 Int64))` is un-equatable because its element is)" —
naming only ONE list level, so it understates where the descent bottoms out (the innermost `->` sits under
two `List` wrappers, not one). The pin's BEHAVIOR is correct (CDZ0216 rejects either way — a `List` of an
un-equatable element is un-equatable at any depth); only the doc's type annotation is off by one nesting
level. LOW/doc-accuracy.

Fix (breaker's call): reword to `List (List (-> Int64 Int64))` (or "a list whose element is itself a list
of closures"), so the explanation matches the two-level input. Batchable with any other 07-type-system
prose touch — no rush, no CI value on its own. Corpus/breaker zone.
