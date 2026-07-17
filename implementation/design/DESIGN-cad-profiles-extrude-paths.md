# DESIGN — CAD 2D profiles, extrusions, paths & splines (capability build)

*2026-07-17. Operator directive (verbatim): "We need to support LINEAR and RADIAL EXTRUSIONS. In my rsolid
library I had a bunch of support for SPLINES and PATHS that would also be great to have. The current form
is fine for a starting point but it'd be hard to do any real cading in it."*

> **STATUS: DESIGN — routed to concierge→operator for the representation forks below.** No code changed.
> Grounded against the current exact model (`implementation/cad/src/exact.cdz`) + the manifold
> `CrossSection`/`extrude`/`revolve` API (both native `manifold-csg` and browser `manifold-3d`).

## The gap

Today the model is 3D-primitive CSG only: `Cube`/`Sphere`/`Cylinder` + booleans + transforms (post-`*r`
rename, per the parallel float-delete directive). Real CAD builds 3D from **2D profiles**: draw a
cross-section (rectangle, circle, or an arbitrary polygon/path, possibly with spline segments), then
**extrude** it (linear — profile × height) or **revolve** it (radial — profile swept about an axis). This
adds a whole modeling layer the current vocabulary lacks.

## What manifold gives us (so we don't reinvent geometry)

manifold has a first-class 2D type, **`CrossSection`** (`square`, `circle`, polygon-from-points, plus 2D
booleans), and two lift operators: **`extrude(crossSection, height, …)`** and
**`revolve(crossSection, degrees, segments)`**. So the driver work is a thin map from our profile/extrude
nodes onto these — the geometry kernel already does the hard part. This mirrors how our 3D primitives map
1:1 onto `Manifold::cube/sphere/cylinder`.

## Proposed model additions

A new **2D profile** type alongside `Solid`, and two extrude nodes that lift a profile to a `Solid`:

```
type Profile =                          // a closed 2D region (in the XY plane), exact where it can be
  | Rect(Vec2)                          // an axis-aligned rectangle of full (w, h)
  | Circle(Rational)                    // a disc of the given radius (segment-approximated at the mesh edge)
  | Polygon(List Vec2)                  // an explicit closed polygon — exact vertices (Rational)
  | PathProfile(Path)                   // a region bounded by a Path (below), for spline/curve outlines

type Solid =                            // (existing, extended)
  | …                                   // Cube/Sphere/Cylinder/booleans/transforms
  | ExtrudeLinear(Profile, Rational)    // profile × height along +z (exact height)
  | Revolve(Profile, Rational)          // profile swept `degrees` about the y-axis (radial)
```

`Vec2` = `V2(Rational, Rational)` (the 2D analogue of `Vec3`).

## Paths & splines — the exactness fork (route to operator)

A **Path** is a sequence of segments describing a 2D outline:

```
type Path =
  | MoveTo(Vec2)
  | LineTo(Vec2)                         // straight segment — EXACT over Rational
  | CubicBezier(Vec2, Vec2, Vec2)        // control pts + endpoint — the spline case
  | …                                    // (Arc, QuadBezier as follow-ups)
```

🔴 **FORK 1 — spline exactness.** A straight `LineTo` polygon is fully exact over Rational. But a
**Bézier/spline point-samples** to a polygon at parameter values `t ∈ [0,1]` via `t²`, `t³`, `(1−t)³` —
which are exact *rational* for rational `t`, BUT the sampled polygon is only an *approximation* of the
true curve (the curve itself is transcendental-free but the tessellation density is a choice). So splines
are **exactly representable as control points (Rational)** but **sampled to a Rational polygon** at a
chosen segment count before meshing — no float needed for the sampling itself, only a
tessellation-count choice (like `Circle`/`Sphere`'s segment count today). *Recommend: store spline
control points exactly (Rational); sample to an N-segment Rational polygon at the driver, N a render
parameter — consistent with how `Circle`/`Sphere` already tessellate.* Confirm this is the wanted model
(vs. a float spline).

🔴 **FORK 2 — revolve angle.** `Revolve(profile, degrees)` for a *full* 360° revolve is exact (no partial
angle). A **partial** revolve (e.g. 90°) needs the profile positioned at an arc — but manifold's
`revolve(cs, degrees, segments)` handles the sweep internally (we pass degrees + segment count), so the
*model* just carries the `degrees` as a Rational and the driver hands it to manifold. Like rotation, the
*angular tessellation* is f64 inside manifold — but our model stays exact (degrees as Rational). *Recommend:
`Revolve` carries `degrees: Rational`; the f64 is confined to manifold's internal sweep, same boundary as
`Sphere`/`Cylinder` tessellation.* Confirm.

🔴 **FORK 3 — Path/spline API surface.** rsolid's exact path/spline API is the reference — shape the
`Path` constructors (LineTo/CubicBezier/Arc/…) + the profile-from-path helper to match its ergonomics.
*Need the operator's rsolid path API as the target* (or a pointer to it) so the surface matches what they
already like.

## Bounding box

`ExtrudeLinear(p, h)` bbox = (profile's 2D bbox) × [0, h]. `Revolve(p, 360)` bbox = a torus-like envelope
from the profile's max radius. Profile bbox folds over the polygon vertices (exact, Rational — the same
min/max fold `bounding-box` already does for 3D). So the exact bbox analysis extends cleanly.

## Rollout (each stage gates green; capability grows incrementally)

1. **`Vec2` + `Profile` (Rect/Circle/Polygon)** + the profile bbox fold. Model-only, exact.
2. **`ExtrudeLinear`** — model node + driver maps to `CrossSection.extrude` (native + browser) + bbox.
3. **`Revolve`** — model node + driver maps to `CrossSection.revolve` + bbox.
4. **`Path` (LineTo/MoveTo)** + `PathProfile` — exact polygonal paths first.
5. **`CubicBezier` splines** — control points exact, sampled to a Rational polygon at N segments.
6. **more worked examples** exercising each (the "more than one example" ask), + a guide showcase.

## Open questions for the operator

1. FORK 1 — spline model: exact control points + sampled Rational polygon (recommended), confirm?
2. FORK 3 — a pointer to rsolid's path/spline API to match its surface?
3. Priority of this capability build vs. the float-delete/prelude-import foundation (P-A/P-B)?

No code until the forks are ruled; on a green-light I ship stage 1 (`Vec2`+`Profile`) first.
