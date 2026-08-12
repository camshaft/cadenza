# 2026-08-12 tuple of two collections (tick 1334, base post-239 trunk fe766af41)

- `tug1.sexp` — handler state `(tuple (List Int64) (Map Int64 Int64))`: pushl/putm
  each advance ONE half through the rebuild-the-tuple idiom, cross reads BOTH halves
  in one answer (list head → map lookup, nested Option-matches). Distinct from bwa1
  (scalar tuple) and rmp1 (record{Map,cnt} — record field access); this is the
  positional-tuple twin with TWO heap collections and a cross-half data dependency.
  Seed list needs the `(: (list) (List Int64))` annotation. PASS ×3 (112062/112002).
