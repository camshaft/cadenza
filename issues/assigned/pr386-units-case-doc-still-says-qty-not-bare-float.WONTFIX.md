# PR review comment — mirrored from GitHub PR #386 (Copilot inline)

- **PR:** #386 (MERGED)
- **File:** `spec/semantics/18-units-of-measure.sexp:731`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589903928
- **Link:** https://github.com/camshaft/cadenza/pull/386#discussion_r3589903928

## Comment (verbatim)
> This case's doc string still claims `Unit.in` returns `(Qty Float64 meter)`, but the expected output was changed to a bare `Float64` (3000.0). Update the doc text to match the unwrapping behavior.

## Liaison triage — CONFIRMED against trunk
Confirmed: the "Unit.in converts a quantity to a chosen larger unit (Float)" case has
`(output (: 3000.0 Float64))` (a bare Float64) but its doc still says "the result is
`(Qty Float64 meter)`". This is the QTY-UNWRAP design change (see backlog design-qty-unwrap /
[[calculator-repl-design]] — `as/in <unit>` unwraps to a bare number). Corpus doc-accuracy fix; route
to `corpus-bugfix` PM. Fix on `trunk`. Quote + link in queue file.
