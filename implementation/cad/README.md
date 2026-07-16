# Cadenza CAD — an OpenSCAD-like solid-modeling library (GH #400)

Describe a parametric 3D solid **in Cadenza code** and render it to a printable/viewable mesh. This
directory is the pure-Cadenza **model layer**: a recursive `Solid` CSG tree (the port of
[camshaft/rsolid](https://github.com/camshaft/rsolid)'s operation-tree) built entirely from ordinary
Cadenza data — sums, recursion, and `Float64`. It needs **zero compiler or runtime change**: a CSG model
*is* a Cadenza value, which is exactly what the language's sums + records + recursion are for.

The mesh backend + render surfaces are per-surface **drivers** that consume this tree as data, kept
separate from the model:
- **native** — `implementation/seed/crates/cdz-cad` (a `manifold-csg` driver): mesh → STL / glTF + bounds.
- **browser** — a `/cad` three.js live preview (planned; `manifold-3d`).

## The model — `src/solid.cdz`

```
type Solid =
  | Empty | Cube(Vec3) | Sphere(Float64) | Cylinder(Float64, Float64)   -- leaves (primitives)
  | Union(Solid, Solid) | Difference(Solid, Solid) | Intersection(Solid, Solid)  -- boolean nodes
  | Translate(Vec3, Solid) | Rotate(Vec3, Solid) | Scale(Vec3, Solid)   -- transform nodes
```

`Vec3` is a single-variant sum `V3(x, y, z)` (all `Float64`). Everyday vocabulary:

| kind | functions |
|---|---|
| primitives | `cube(w,d,h)`, `cube-uniform(s)`, `sphere(r)`, `cylinder(h,r)`, `empty()` |
| vectors | `v3(x,y,z)`, `v3-zero()`, `v3-splat(s)`, `v3-add`, `v3-mul`, `v3-eq`, `vx`/`vy`/`vz` |
| booleans | `union`/`difference`/`intersection` (binary); `union-all`/`intersection-all`/`difference-all` (n-ary over a `List`, via fold) |
| transforms | `translate(by, of)`, `rotate(deg, of)`, `scale(by, of)` |
| normalize | `simplify(s)` — absorb the `Empty` identity (union Empty x = x, intersection _ Empty = Empty, …) |
| analyses | `count-nodes`, `leaf-count`, `depth`, `is-empty` |

Primitives are centred at the origin; `cube w d h` is an axis-aligned box, `cylinder h r` runs along +z.

### Example — the marquee plate

```
def plate(w: Float64, d: Float64, t: Float64, r: Float64) =
  difference-all(cube(w, d, t),
    [ translate(v3(w * 0.25, d * 0.5, 0.0), cylinder(t, r)),
      translate(v3(w * 0.75, d * 0.5, 0.0), cylinder(t, r)) ])
```

A rectangular plate with two bolt holes — `difference-all` cuts each hole from the base. More worked
models live in `src/examples.cdz`: `washer`, `capsule`, `cube-row`, `radial-pattern` (a circular fold —
the gear/bolt-circle pattern), `gear`, `tube`, `hollow-box`, and `hole-grid` (a nested double recursion).

## Test

```
cdz test implementation/cad      # run the @test suite (50 cases: shape/fold/simplify invariants)
```

`Project.cdz` is the manifest (well-known top-level `def`s — `name`/`modules`/`tests`, globs allowed),
the same shape `implementation/compiler-ml` uses. The suite is gated fleet-wide by the `cad-tests` CI job.

## Render (native)

The model crosses the boundary as canonical s-expr text; the `cdz-cad` driver meshes it:

```
cdz run model.cdz | cdz-cad - -o out.stl        # binary STL (printer) — `--ascii` for text STL
cdz run model.cdz | cdz-cad - -o out.glb        # binary glTF (viewers)
cdz run model.cdz | cdz-cad - --info            # inspect: triangle/vertex counts + bounding box
```

See `implementation/seed/crates/cdz-cad/README.md` for the driver.

## Notes for contributors

This library is also a real **stress test** of the language (recursion + sums + `Float64` at scale) —
if a model can't be expressed cleanly, **report the gap** (file a repro), don't contort around it. Two
findings so far: the ML surface parses no named/inline record type (so nominal products are single-variant
sums), and a recursive `Float64`-comparison fold hits an unimplemented runtime scalar-`Float64`-`==`
(so a language-side `bounding-box` fold is deferred — the native driver's `bounds` covers it meanwhile).
