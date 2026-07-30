# PR#935 review comment — 19-sets NaN-tuple doc example drops the Float64 annotation (corpus-bugfix)

Mirrored from GitHub PR#935 review comment (Copilot), id `3685613221`.
File: `spec/semantics/19-sets.sexp:2878` — corpus doc → corpus-bugfix. Blame `e8ed7b8c3`
"corpus(5 files): 5-pin drain AE — … NaN-width dedup …".

## Comment (verbatim)

- (id 3685613221, 19-sets.sexp:2878) "The doc string's example tuple doesn't match the actual test
  input: the code uses a Float64-typed 1.5 (`(: 1.5 Float64)`), but the prose says `(tuple Float32.nan
  1.5)`. Keeping the doc example exact helps avoid confusion about the intended mixed-width element."

## Liaison verification (confirmed on trunk e8ed7b8c3)

Case "a mixed-width float tuple with a computed f32 NaN dedupes against the canonical spelling". Doc:
"deduping against `(tuple Float32.nan 1.5)`". But the `(input …)` uses `(tuple Float32.nan (: 1.5
Float64))` — the `1.5` is EXPLICITLY `Float64`-annotated. This matters BECAUSE the case's whole point is
the MIXED-WIDTH element (f32 NaN leaf + f64 leaf): the bare `1.5` in the doc obscures that the second leaf
is deliberately f64. Fix: write the doc example as `(tuple Float32.nan (: 1.5 Float64))` to match the
input exactly. Doc-only, pin correct.

Owner: **corpus-bugfix** (`spec/semantics/19-sets.sexp`; `e8ed7b8c3`). Restore the `(: 1.5 Float64)`
annotation in the doc example.
