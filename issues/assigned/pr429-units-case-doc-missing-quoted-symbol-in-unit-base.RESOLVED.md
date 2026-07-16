# PR review comment — mirrored from GitHub PR #429 (Copilot inline)

- **PR:** #429 "fleet: fifty-third batch (…, unit-join, …)" (MERGED)
- **File:** `spec/semantics/18-units-of-measure.sexp:621`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592186522
- **Link:** https://github.com/camshaft/cadenza/pull/429#discussion_r3592186522

## Comment (verbatim)
> In this doc string you refer to the rendered type as `(Qty Int64 (Unit.base meter))`, but the unit surface form elsewhere (and in `Unit::render`) includes the quoted symbol: `(Unit.base #"meter")`. As written, the snippet is not a valid/consistent unit surface form and may confuse readers about what actually renders.

## Liaison triage — CONFIRMED against trunk
Confirmed: the case doc (18-units-of-measure.sexp:621) says both branches "RENDER to
`(Qty Int64 (Unit.base meter))`", but the actual surface form — and `Unit::render` — quotes the symbol:
`(Unit.base #"meter")`. So the doc snippet isn't a valid/consistent unit surface form. Doc-accuracy in
the quantity corpus. FIX: use `#"meter"` in the doc to match what renders. Quantity territory (v-quantity
owns unit rendering + these Qty cases). Fix on `trunk`. Quote + link in queue file.
