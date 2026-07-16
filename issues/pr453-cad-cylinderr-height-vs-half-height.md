# PR review comment — mirrored from GitHub PR #453 (Copilot inline)

- **PR:** #453 "fleet: seventy-third batch (…, cad R3 exact model, …)" (MERGED)
- **File:** `implementation/cad/src/exact.cdz:30` (`Cylinderr`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593103043
- **Link:** https://github.com/camshaft/cadenza/pull/453#discussion_r3593103043

## Comment (verbatim)
> `Solidr.Cylinderr` is documented as storing `(height, radius)`, but the rest of the file treats the first field as a half-height: `bounding-box` uses `z ∈ [-h, h]` and the `cylinder-box-is-diameter-by-height` test expects size `2h`. This comment should match the actual representation to avoid consumers passing a full height by mistake.

## Liaison triage — CONFIRMED against trunk — same class as pr449 Cuber
Confirmed: `Cylinderr` is documented as `(height, radius)` (full height), but `bounding-box` uses
`z ∈ [-h, h]` (first field = HALF-height) and the `cylinder-box-is-diameter-by-height` test expects
size `2h`. So a consumer passing a FULL height gets a 2×-tall cylinder. This is the SECOND instance of
the exact-CSG size/half-extent doc mismatch (cf. pr449 `Cuber` size-vs-half-extents) — the exact.cdz
primitives consistently store half-extents/half-heights but document them as full size. FIX: make the
doc match the half-height representation (or halve in bounding-box to match a full-height doc) AND
reconcile the whole exact.cdz primitive set (Cuber, Cylinderr, …) before the R-series merge with the
main `Solid`. CAD territory (v-cad). Fix on `trunk`. Quote + link in queue file.
