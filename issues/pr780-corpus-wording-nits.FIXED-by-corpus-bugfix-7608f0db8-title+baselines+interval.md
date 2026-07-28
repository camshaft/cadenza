# PR#780 review comments — corpus wording nits (strings case title/doc + binding interval self-contradiction)

Mirrored from GitHub PR review comments (Copilot), ids `3629690353`, `3629690388`, `3629690421`,
`3629690454`, `3629690487`.
PR: https://github.com/camshaft/cadenza/pull/780 (batch-staging; fixes belong on trunk)
Locations: `spec/semantics/13-strings.sexp:1626`, `spec/semantics/02-binding-and-control.sexp:292`,
+ 3 gate-baseline entries (`.gate-baseline:118`, `.gate-baseline-rust:118`, `.gate-baseline-rust-async:117`).

All LOW priority / readability. Pinned semantics are correct in both cases; these are doc/title wording.

## Comments (verbatim)

- (id 3629690353, 13-strings.sexp:1626) "The case title is missing an article ('into the to-bytes
  output'), and the doc's `(byte-of-f, 0xC3)` wording is unclear/atypical (it's the byte 0x66 for
  'f'). This makes the title/doc harder to read and search for consistently in gate baselines."
- (ids 3629690388/…421/…454, the 3 gate baselines) "This gate-baseline entry should match the case
  title string exactly after fixing the grammar in `13-strings.sexp` ('into the to-bytes output')."
- (id 3629690487, 02-binding-and-control.sexp:292) "In the doc for the soundness twin, 'x∈[1,
  i64::MAX], NO upper bound' is internally inconsistent (the interval as written does include an upper
  bound). This wording is confusing about what information is and isn't provided by the refinement."

## Liaison verification (CONFIRMED on trunk — all minor)

1. 13-strings.sexp:1625 title `"a Bytes.slice into to-bytes output decodes …"` — missing an article
   ("into THE to-bytes output"). The doc (line ~1626) says the a=2 window is `(byte-of-f, 0xC3)` —
   mixes a symbolic "byte-of-f" with a hex literal for é's lead byte; clearer as `(0x66 'f', 0xC3)` or
   "('f' = 0x66, then é's lead byte 0xC3)". Landed `31d620efb`.
2. The 3 gate-baseline lines are CONTINGENT: they only need editing IF the case title changes (the
   baseline title must byte-match the case title). Not independent defects — they travel with fix #1.
3. 02-binding-and-control.sexp:292 — "`(> x 0)` (x∈[1, i64::MAX], NO upper bound)": the interval
   `[1, i64::MAX]` DOES have an upper bound; the intent is "no upper bound tighter than the type max".
   Reword e.g. "x∈[1, i64::MAX] — bounded below by the refinement, only the type max above". Landed
   `f2ef2ddf0` (value-facts slice 6c).

If the title in #1 is changed, remember: a corpus `.sexp` title edit must be mirrored in ALL THREE
gate baselines (that's what comments 2-4 are about) AND needs `xtask roundtrip` +
`cargo test -p cadenza-syntax --test corpus_roundtrip`.

Owner: semantics corpus (v-compiler-ml / the pin authors `31d620efb` + `f2ef2ddf0`). Filed to
corpus-bugfix PM as LOW to bundle. Not worth 5 separate messages — the 3 baseline ones are just the
title-sync of fix #1.
