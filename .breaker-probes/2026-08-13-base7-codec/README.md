# 2026-08-13 base-7 codec state (tick 1424)

- `bas1.sexp` — encode peels a value's base-7 digits MSB-first via recursive
  List.prepend (peel builds little→big by prepending), appends the run to the
  digit-list state (cat walk); decode positional-folds the WHOLE accumulated
  run and clears. The round-trip law face: two concatenated encodes decode as
  POSITIONAL COMPOSITION (enc 10 + enc 3 → digits [1,3,3] → 73 = 10·7+3), not
  as the values' sum — a digit-boundary bug shifts the composition. dgn1/dg1
  peel digits of the STATE; here the peel is of the ARG and the state is the
  digit STREAM. PASS ×3 (230733050/233393050).
