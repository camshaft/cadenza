# PR#839 review comments — compound-types select-join cases: doc says "loser's length re-read" but code re-reads a FIXED handle

Mirrored from GitHub PR review comments (Copilot), ids `3637316510`, `3637316531`.
PR: https://github.com/camshaft/cadenza/pull/839 (merged; fix belongs on trunk)
Location: `spec/semantics/05-compound-types.sexp` — the list if-select case (~9502, also :9516) and the
Map if-select case (~9532, also :9538). Both landed `f6c09b1ac` (#20 escape-shape, RRB + CHAMP).

## Comments (verbatim)

- (id 3637316510, :9502) "The doc claims the selected list's loser has its length re-read after the
  select join, but the current mode 2/3 walkthrough (and the current code) only re-reads `List.len
  out`. If the intent is to pin both winner/loser liveness regardless of which side wins, the mode 2/3
  explanation (and outputs) should reflect re-reading the actual loser length. This issue also appears
  on line 9516."
- (id 3637316531, :9532) "The doc says the loser map's len is re-read after the `if`-select join, but
  the mode 2 walkthrough/output uses `Map.len m1` (the winner when pick=m1). If the intent is to
  always re-read the loser, the mode 2 explanation/output should be updated to reflect that. This
  issue also appears on line 9538."

## Liaison verification (CONFIRMED on trunk — doc-accuracy, behavior-neutral)

- List case (~9515): the escape re-reads `(List.len out)` — a FIXED handle. The doc (~9498) says "the
  loser's length is re-read after its last select use." `out` is the LOSER only in mode 1 (pick=s);
  in mode 2 (pick=out) `out` is the WINNER, so the "loser's length re-read" claim mis-describes modes
  2/3.
- Map case (~9539): re-reads `(Map.len m1)` — fixed. Doc (~9528) says "the loser's len is re-read."
  `m1` is the loser in modes 1/3 (pick=m2) but the WINNER in mode 2 (pick=m1).

The pinned SEMANTICS/outputs are correct (1567/117/7 and 1004/101/1001) — only the DOC prose overclaims
"loser's length" when the code unconditionally re-reads a fixed handle (which is sometimes the winner).
Either (a) reword the doc to "a fixed handle's (out / m1) length is re-read after the join — the winner
in some modes, the loser in others; both must survive regardless", or (b) if the INTENT was to pin the
loser specifically, change the code to re-read the non-picked side (would change outputs → update
`(output …)` + baselines). Option (a) is the behavior-neutral doc fix and likely what's wanted (the pin
is about BOTH handles surviving the join).

LOW priority / doc-accuracy. `.sexp` edit → `xtask roundtrip` + `cargo test -p cadenza-syntax --test
corpus_roundtrip` (option (a) is doc-only, no output/baseline change). Owner: **corpus-bugfix**
(spec/semantics lane; case landed `f6c09b1ac`). Routed as an issue.
