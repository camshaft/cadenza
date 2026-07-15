# Vertical-ready brief — Cadenza CAD (solid modeling + manifold render)

**Design doc:** `implementation/design/DESIGN-cad-solid-modeling.md` (landed on trunk).
**Source:** GH #400 (operator), shaped by the `design-cad-scad` design agent 2026-07-15.

## What it is
An OpenSCAD-like CAD tool in Cadenza: describe parametric 3D solids in code + render them. Port
`camshaft/rsolid`'s operation-tree to a Cadenza recursive `Solid` sum (pure library), render via a
per-surface **manifold** mesh driver. Two surfaces off one core (calculator precedent): native
`cdz cad` file-out + a guide `/cad` live three.js preview.

## Subsystem(s)
**Mixed — mostly a Cadenza library + `cdz`/guide tooling, with ONE compiler-adjacent seam (the manifold
host binding).** Suggested `vertical` area: **cad** (a new library + tooling vertical), NOT a compiler
vertical — the model DSL needs zero compiler change. If the operator later chooses D2=effect (manifold
as a Cadenza effect), that seam touches rcdzc's host-effect surface.

## First increment (G1 — start here)
**The `Solid` model library, pure Cadenza, no mesh dependency.** A recursive `Solid` sum (Cube/Sphere/
Cylinder leaves; Union/Difference/Intersection nodes; Translate/Rotate/Scale transforms), `Vec3` + 4×4
affine-matrix helpers (Float64), constructor/combinator `def`s (binary + n-ary via `fold`). Gate:
pure-data unit tests (build a tree, assert shape; a `bounding-box`/`count-nodes` fold runs e2e under
wasmtime). Zero compiler change. This is the "port rsolid's tree" milestone + a real recursion/sums/
records/floats stress test — **report (don't work around) any language gap** as a finding.

## Full increment path (see doc §5)
G0 design+skeleton (done: doc landed) → **G1 model library** → G2 native mesh driver (`manifold-csg`) +
`cdz cad` file-out → G3 browser `/cad` route + three.js preview (`manifold-3d`) → G4 worked examples from
the operator's printing repo → G5 interface polish (⚑ D1-gated: solid operator sugar + param sliders).

## Two open operator decisions (defaults chosen — build proceeds; concierge holds the asks)
- **D1 surface sugar:** plain applicative surface first; `+`/`-`/`*` = union/difference/intersection sugar
  is a later increment (G5). Default: no new operators in the first cut.
- **D2 tree→driver seam:** **B1** (Cadenza produces the CSG tree as data, the per-surface driver meshes
  it) — recommended default; B2 (manifold-as-effect) is the documented alternative, heavier.

## Key finding to carry into the build
**manifold is NOT a Cadenza WIT peer.** `manifold-3d` is Emscripten WASM + JS glue (not a component over
the shared value-heap runtime); `manifold-csg` is the native Rust crate. Bind manifold as a per-surface
mesh driver consuming the CSG tree as data — NOT via `(bind …)` peer interop. (Corrects the intake's
"fits the peer-interop story" framing.)
