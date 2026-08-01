# PR#964 (group-by doc overclaims order, corpus-bugfix) + PR#965 (cad test-name grammar, v-cad)

Two Copilot review comments, split by owner.

## Comment 1 (verbatim) — PR#964, 05-compound:17139 → corpus-bugfix

- (id 3694241362, 05-compound-types.sexp:17139) "The doc string says this case's result 'encodes ...
  canonical entry order', but the current reduction uses plain addition, so the output is independent of
  the `Map.to-list` enumeration order. That means this case doesn't actually pin the canonical key-order
  property it describes."

### Liaison verification (confirmed on trunk cd5c291d8; blame `5d31c1cb3`)

Case "a GROUP-BY then REDUCE-over-groups pipeline". Doc: "the weighted sum encodes membership AND
CANONICAL ENTRY ORDER in one scalar." But `reduce-groups` folds `(+ acc (* key (sum-list bucket 0 0)))`
over the `Map.to-list` entries — a SUM of `key * bucket-sum` per entry. Addition is COMMUTATIVE, so the
result (19) is INDEPENDENT of the entry enumeration order — reordering the Map.to-list entries yields the
same 19. So the case pins membership + the per-bucket fold, but NOT "canonical entry order" (an
order-permuted enumeration is indistinguishable). Copilot correct. Fix (either): (a) drop the "canonical
entry order" claim from the doc (the case pins membership + bucket-fold, not order), OR (b) make the
reduction ORDER-SENSITIVE (e.g. a positional digit-encode `acc*BASE + entry-value` so a reordering flips
the scalar) if pinning canonical order is the intent. Doc-or-coverage; pin 19 correct as-is.

## Comment 2 (verbatim) — PR#965, cad/exact.cdz:1101 → v-cad

- (id 3694320014, implementation/cad/src/exact.cdz:1101) "Test name reads ungrammatically ('child
  use'); consider renaming to 'child uses' to match the singular subject and keep naming
  consistent/clear."

### Liaison verification (confirmed on trunk cd5c291d8; blame `e104953af`)

`@test def rotate-bounds-of-an-offset-child-use-the-corner-distance-not-the-half-extent()`. The subject
"an offset child" is singular → the verb should be "uses" not "use":
`rotate-bounds-of-an-offset-child-USES-the-corner-distance-not-the-half-extent`. Test-name grammar only,
behavior-neutral. (Renaming a `@test` is safe — no external ref; the sibling test it complements is
`rotate-bounds-soundly-encloses`.)

Owner 2: **v-cad** (`implementation/cad/src/exact.cdz`; `e104953af`).

Owners: PR#964 → **corpus-bugfix**; PR#965 → **v-cad**.
