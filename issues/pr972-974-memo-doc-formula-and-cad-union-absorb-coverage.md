# PR#972 (memo doc formula, corpus-bugfix) + PR#974 (cad union-absorb coverage, v-cad)

Two Copilot review comments, split by owner.

## Comment 1 (verbatim) — PR#972, 05-compound:17166 → corpus-bugfix

- (id 3694523268, 05-compound-types.sexp:17166) "The docstring's example formula is slightly
  inconsistent with the actual expression used in the case: it says '(* 100 v) + Map.len' (missing the
  `memo` argument and not matching the `(+ (* 100 v) (Map.len memo))` structure). Align the doc's formula
  with the code so it's unambiguous what is being pinned."

### Liaison verification (confirmed on trunk 8693eb3c5; blame `4db2c7380`, the PR#971 fix)

The PR#971 fix (`4db2c7380`) correctly reworked the case to `(+ (* 100 v) (Map.len memo)) = 676521`. The
doc now says "The output encodes the EXACT memo size beside the value ((* 100 v) + Map.len = 6765*100 +
21 = 676521)". Copilot's nit: the doc's inline formula "(* 100 v) + Map.len" drops the `memo` arg
(`Map.len memo`) and uses infix `+` vs the code's `(+ … (Map.len memo))`. Minor doc-code formula
mismatch. Align to `(+ (* 100 v) (Map.len memo))`. Doc-only, pin 676521 correct.

Owner 1: **corpus-bugfix** (`spec/semantics/05-compound-types.sexp`; `4db2c7380`).

## Comment 2 (verbatim) — PR#974, cad/exact.cdz:864 → v-cad

- (id 3694585858, implementation/cad/src/exact.cdz:864) "This test now only checks that `sl`/`sr` are
  `Solid.Cube(_)`, but it doesn't assert the cube is unchanged (or even non-degenerate). A bug that
  returns some other cube (e.g. `Cube(v3(0,0,0))`) would still pass. Consider keeping the new structural
  check *and* reintroducing an extent/payload assertion so the test pins both 'Union prunes Empty' and
  'the surviving cube is preserved.'"

### Liaison verification (confirmed on trunk 8693eb3c5; blame `63d95977c`)

The `simplify-r` union-absorb test (`63d95977c` "strengthen … to pin the left-operand prune
structurally") now matches `sl`/`sr` against `Solid.Cube(_)` (a wildcard payload). Copilot's point: the
`Cube(_)` wildcard confirms the survivor is A cube but NOT that it's the ORIGINAL cube — a bug returning
`Cube(v3(0,0,0))` (a degenerate/zeroed cube) or a differently-sized cube would still match `Cube(_)` and
pass. So the test pins "Union prunes Empty → a Cube survives" but not "the surviving cube is PRESERVED
(right extent/payload)". Fix (Copilot's, sound): keep the structural `Cube(_)` check AND assert the cube's
extent/payload equals the input's (a value/extent check), pinning both prune + preservation.
Test-coverage; behavior-neutral.

Owner 2: **v-cad** (`implementation/cad/src/exact.cdz`; `63d95977c`).

Owners: PR#972 → **corpus-bugfix**; PR#974 → **v-cad**.
