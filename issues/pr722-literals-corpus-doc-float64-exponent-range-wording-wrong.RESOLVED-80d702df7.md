# PR#722 review comment — 01-literals.sexp docstring: "top of Float64 exponent range" is numerically wrong

Mirrored from GitHub PR review comment (Copilot), id `3619635072`.
PR: https://github.com/camshaft/cadenza/pull/722 (merged; fix still belongs on trunk)
Location: `spec/semantics/01-literals.sexp:424`

## Comment (verbatim)

> The docstring claims `3.4028235e38` is "at the top of the finite Float64 exponent range", but
> Float64's finite range extends to ~1.8e308. This value is near the Float32 max and only a
> large-magnitude *within* Float64; updating the wording avoids embedding a numerically incorrect
> statement in the corpus documentation.

## Liaison verification (CONFIRMED on trunk)

Case "a float at the top of the binary64 exponent range renders its full decimal expansion" (line ~423).
Its `doc` opens: "…called with 3.4028235e38 — a value **at the top of the finite Float64 exponent
range** (near the f32 max, well inside binary64's ~1.8e308 ceiling)…". The lead phrase "top of the
finite Float64 exponent range" is numerically FALSE (3.4e38 ≪ 1.8e308), and it directly contradicts
its OWN parenthetical ("near the f32 max, well inside binary64's ~1.8e308 ceiling"). The case *title*
also says "top of the binary64 exponent range".

Fix: reword the lead phrase + the title to describe it accurately — e.g. "a large-magnitude finite
Float64 near the Float32 max" — keeping the accurate parenthetical. The VALUE assertion
(340282349999999991754788743781432688640.0, the full decimal expansion) is correct and unchanged;
this is a doc-wording fix only, no semantics change.

Landed via `96fca0b1f` ("corpus: pin the top-of-exponent Float64 value render (full decimal
expansion)"). Semantics corpus doc → filing to corpus-bugfix PM to route to the pin's owner.
NOTE for PM: a corpus `.sexp` doc-string edit needs the roundtrip gates
(`xtask roundtrip` + `cargo test -p cadenza-syntax --test corpus_roundtrip`).
