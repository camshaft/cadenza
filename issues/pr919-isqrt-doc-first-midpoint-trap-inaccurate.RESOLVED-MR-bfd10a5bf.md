# PR#919 review comment — isqrt doc "first midpoint square" trap claim inaccurate (corpus-bugfix)

Mirrored from GitHub PR#919 review comment (Copilot), id `3684305642`.
File: `spec/semantics/06-numeric-model.sexp:7813` — corpus doc → corpus-bugfix. Blame `dc830b2b7`
"corpus(numeric): 4-pin drain Q — … isqrt at the checked ceiling".

## Comment (verbatim)

- (id 3684305642, 06-numeric-model.sexp:7813) "The doc string claims that increasing `hi` by 1 would
  trap 'at the first midpoint square', but the first midpoint with `hi=3037000500` would be ~1.5e9 and
  its square would not overflow. The overflow trap would occur later when the search evaluates a
  midpoint of 3037000500 (or otherwise exceeds `isqrt(MAX)`)."

## Liaison verification (confirmed on trunk 5dfc74b9e)

Case "a binary-search isqrt with an overflow-safe hi bound computes at i64::MAX". Doc: "…a hi one larger
would trap at the FIRST midpoint square." The isqrt-go binary search starts `lo` small (0) and `hi` at
the cap; the FIRST `mid = (lo+hi)/2` with `hi=3037000500` is ~1.5e9, and `mid*mid` ≈ 2.25e18 < i64::MAX
(~9.22e18) — NO overflow at the first midpoint. The checked-arith overflow only trips LATER, once the
search narrows `lo` up so a `mid` approaches 3037000500 (whose square 9.22e18+ exceeds i64::MAX). So
"first midpoint square" is inaccurate — the trap is at a LATER midpoint near `isqrt(MAX)+1`. Fix: reword
to "would trap at a later midpoint (once mid exceeds isqrt(MAX))", or drop "first". Doc-only, pin correct.

Owner: **corpus-bugfix** (`spec/semantics/06-numeric-model.sexp`; `dc830b2b7`). Reword the trap-timing
claim.
