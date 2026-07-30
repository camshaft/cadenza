# PR#937 review comment — 15-rows doc renders element type as (Record (x)(t)) but input is (Record (x Int64)(t Int64)) (corpus-bugfix)

Mirrored from GitHub PR#937 review comment (Copilot), id `3686156951`.
File: `spec/semantics/15-rows-and-open-sums.sexp:1476` — corpus doc → corpus-bugfix. Blame `d3c7c0657`
"corpus(3 files): 3-pin drain AG — … collection-borne row projection …".

## Comment (verbatim)

- (id 3686156951, 15-rows-and-open-sums.sexp:1476) "The new case's doc string describes the collection
  element type as `(List (Record (x)(t)))`, but the actual program uses `(List (Record (x Int64) (t
  Int64)))`. This mismatch makes the case harder to understand and can mislead future readers when
  searching for pinned behaviors by type shape."

## Liaison verification (confirmed on trunk 994ea6a0d)

Case "an open-row projection reads list-element records in a fold AND a wider record at another site".
Doc: "…get-x applied to `(List (Record (x)(t)))` elements inside a recursive fold…". The `(x)(t)`
shorthand omits the field TYPES, but the input's `sum-xs` param is typed `(: rs (List (Record (x Int64)
(t Int64))))`. So the doc's `(Record (x)(t))` doesn't match the actual `(Record (x Int64) (t Int64))` —
inconsistent and defeats a type-shape search. Fix: write the doc type as `(List (Record (x Int64) (t
Int64)))` to match the input. Doc-only, pin correct.

Owner: **corpus-bugfix** (`spec/semantics/15-rows-and-open-sums.sexp`; `d3c7c0657`). Make the doc type
exact.
