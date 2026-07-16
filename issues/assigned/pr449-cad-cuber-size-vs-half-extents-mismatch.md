# PR review comment — mirrored from GitHub PR #449 (Copilot inline)

- **PR:** #449 "fleet: sixty-ninth batch (…, cad R1 exact-Rational geometry, …)" (MERGED)
- **File:** `implementation/cad/src/exact.cdz:26` (`Cuber` / `bounding-box`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592901846
- **Link:** https://github.com/camshaft/cadenza/pull/449#discussion_r3592901846

## Comment (verbatim)
> `Solidr.Cuber`'s parameter is described as a size `(w, d, h)`, but `bounding-box` treats it as half-extents (min = `-w`, max = `+w`). This differs from `implementation/cad/src/solid.cdz` where `Cube(Vec3)` is documented as full size ("cube 2.0 2.0 2.0 is a 2-unit cube"). Clarify `Cuber`'s semantics here to avoid downstream confusion when the models are merged.

## Liaison triage — CONFIRMED against trunk — semantic inconsistency (exact vs main CSG)
Confirmed: `exact.cdz` declares `Cuber(Vec3r)  // an axis-aligned box of the given (w, d, h), centred at
the origin` (reads as FULL size), but per the reviewer the `bounding-box` fold treats the Vec3r as
HALF-EXTENTS (min = -w, max = +w) — so `Cuber(2,2,2)` would be a 4-unit box, contradicting both this doc
AND the sibling `solid.cdz` `Cube(Vec3)` documented as full size ("cube 2.0 2.0 2.0 is a 2-unit cube").
This is a real semantic inconsistency between the exact-CSG `Cuber` and the main `Cube` that will
produce WRONG geometry when the models merge (the R2 migration the code comment mentions). FIX: make
`Cuber` agree with `Cube` (full size — halve in bounding-box) OR clearly re-document `Cuber` as
half-extents AND reconcile before the R2 merge. CAD territory (v-cad). Fix on `trunk`. Quote + link in
queue file.
