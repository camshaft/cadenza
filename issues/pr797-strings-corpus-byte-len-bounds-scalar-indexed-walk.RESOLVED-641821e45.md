# PR#797 review comments — 13-strings corpus cases bound scalar-indexed walks with String.byte-len (should be scalar-len)

Mirrored from GitHub PR review comments (Copilot), ids `3633101038`, `3633101081`.
PR: https://github.com/camshaft/cadenza/pull/797 (batch-staging; fixes belong on trunk)
Location: `spec/semantics/13-strings.sexp:103` (paren-depth `scan`) and `:137` (`split-go`).

## Comments (verbatim)

- (id 3633101038, :103) "`String.at` and `String.slice` are scalar-indexed (per prelude docs), but
  this loop bounds `scan` with `String.byte-len`. That will terminate too late (or hit the `(None _u)`
  arm unexpectedly) for non-ASCII scalars, contradicting the case's own 'scalar walk' description. Use
  `String.scalar-len` for the iteration bound."
- (id 3633101081, :137) "`split-go` iterates with `String.at` (scalar-indexed) and slices with
  `String.slice` (scalar offsets), but the initial `len` is `String.byte-len`. For strings containing
  multibyte scalars, this can drive `i` past the scalar length and either drop the final flush or hit
  the `(None _u)` arm. Use `String.scalar-len` for `len`."

## Liaison verification (CONFIRMED on trunk — real, latent behind ASCII inputs)

Both cases iterate `i` with scalar-indexed `String.at s i` / `String.slice s start i` but seed the
loop bound `len` from `String.byte-len s` (scan: line ~102 `(scan s 0 (String.byte-len s) …)`; split:
line ~137 `(split-go s 0 (String.byte-len s) 0 (list))`). For a multibyte scalar, byte-len > scalar-len,
so `i` runs past the last scalar → the extra iterations hit `String.at`'s `(None _u)` out-of-range arm
(scan returns the -9 sentinel; split's `((None _u) acc)` drops the final flush). Contradicts the cases'
own "scalar walk" / scalar-offset framing.

LATENT: the actual test inputs are ASCII (`"("`, `")"`, `"()"`, and comma-separated ASCII fields), so
byte-len == scalar-len and the cases PASS today — but the pin does NOT exercise the multibyte behavior
it describes, and the code is a latent bug for any non-ASCII input.

Fix (per Copilot): use `String.scalar-len` for the loop bound in BOTH cases. Consider also adding a
multibyte input (e.g. a field/paren string with an "é") so the pin actually covers the scalar-vs-byte
distinction. `.sexp` edit → `xtask roundtrip` + `cargo test -p cadenza-syntax --test corpus_roundtrip`;
if the body changes the OUTPUT for a new multibyte input, update the `(output …)` + baselines.

Owner: **corpus-bugfix** (spec/semantics/*.sexp is corpus-bugfix's lane, NOT v-compiler-ml — the
fleet-send wrong-attribution guard blocks a compiler-ml MR there; see the routing memory). Routed as an
issue.
