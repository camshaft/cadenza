# PR#969 review comment — cad rotate-bounds test name "take" should be "takes" (v-cad)

Mirrored from GitHub PR#969 review comment (Copilot), id `3694453750`.
File: `implementation/cad/src/exact.cdz:1123` — v-cad. Blame `90b19771a` "cad: pin rotate-radius takes
max|corner| including the negative side".

## Comment (verbatim)

- (id 3694453750, implementation/cad/src/exact.cdz:1123) "Test name grammar is inconsistent with nearby
  tests (`...-encloses`, `...-uses`) and reads like a typo: `rotate-bounds-take-...` should be
  `rotate-bounds-takes-...` for clarity/searchability."

## Liaison verification (confirmed on trunk d247bf556)

`@test def rotate-bounds-take-the-max-abs-corner-including-the-negative-side()`. Subject "rotate-bounds"
(and per the doc "`aabbr-rotate-radius` must TAKE `rmax(|lo|,|hi|)`") — the test-name verb should be
"takes" to match the sibling tests (`rotate-bounds-soundly-ENCLOSES`, `rotate-bounds-of-an-offset-child-
USES` [the PR#965 fix]). `take` → `takes`:
`rotate-bounds-TAKES-the-max-abs-corner-including-the-negative-side`. Test-name grammar only,
behavior-neutral (safe rename — no external ref). This is the 3rd in the rotate-bounds test-name family
(PR#965 `use`→`uses`, now `take`→`takes`); v-cad may want to sweep the whole family for verb agreement.

Owner: **v-cad** (`implementation/cad/src/exact.cdz`; `90b19771a`). Rename `take`→`takes`.
