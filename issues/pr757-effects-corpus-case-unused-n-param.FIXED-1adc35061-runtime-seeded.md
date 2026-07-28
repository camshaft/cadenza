# PR#757 review comment — effects corpus case: `main` has an unused `n` parameter (readability)

Mirrored from GitHub PR review comment (Copilot), id `3626031854`.
PR: https://github.com/camshaft/cadenza/pull/757 (merged; fix still belongs on trunk)
Location: `spec/semantics/14-effects-and-handlers.sexp:1080` (case "The next-state twin … `double-up`")

## Comment (verbatim)

> `main` takes an `n` parameter but never uses it (the handler is seeded with the constant `1`), and
> the test call passes `0`. This makes the case harder to read and looks like an accidental leftover
> parameter.
>
> Consider either (a) removing the parameter and calling `main` with no args, or (b) using `n` as the
> handler seed and updating the call accordingly.

## Liaison verification (CONFIRMED on trunk — LOW priority / readability)

The case body:
```
(def (main (: n Int64))
  (handle Tw 1                       ; <- seeded with the literal 1, not n
    ((next (u) s (resume s (double-up s 2))))
    (do (Tw.next unit) (Tw.next unit))))
(export main)))
(call main (: 0 Int64)) (output (: 4 Int64))
```
`n` is declared but never referenced; the handler seed is the constant `1`; the call passes `0`. The
pinned SEMANTICS are correct (output 4), so this is purely a corpus-readability defect — a misleading
leftover parameter in a reference case.

Fix (per Copilot, either works):
- (a) drop the param: `(def (main) (handle Tw 1 …))` + `(call main)`, OR
- (b) make `n` meaningful: `(def (main (: n Int64)) (handle Tw n …))` + `(call main (: 1 Int64))`
  (keeps output 4; option (b) also broadens coverage — a runtime-seeded handler).

LOW priority (nit, no semantics impact). Owner: semantics corpus (effects cases, landed in the
`a3bab62a9`/`a98bf6247`/`476e3c915` effects-corpus series). Filing lightly to corpus-bugfix PM to bundle
with other effects-corpus touch-ups. NOTE: a `.sexp` edit needs `xtask roundtrip` +
`cargo test -p cadenza-syntax --test corpus_roundtrip`; if the case title/count changes, update the
gate baselines.
