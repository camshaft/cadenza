# cdz-cad — the native mesh driver for Cadenza CAD (GH #400)

Turns a Cadenza program that describes a solid (built on the [`implementation/cad`](../../../cad) `Solid`
library) into a **mesh file** — a printable STL or a viewable binary glTF — and reports the model's bounding
box. This is the *native* half of "describe a model in code and see it" (the browser `/cad` preview is the
other surface).

## The pipeline

A Cadenza program's single export crosses the component boundary **already rendered to canonical s-expr
text** (the B1 "render-tree-as-data" seam). `cdz-cad` consumes that text:

```
cdz run model.cdz | cdz-cad - -o out.stl        # from stdin (the run surface's pipe)
cdz-cad model.sexp -o out.glb --segments 64     # from a file
cdz-cad model.sexp --info                        # inspect only: triangle/vertex counts + bounds, no file
```

1. **parse** the rendered `Solid` s-expr into a CSG tree ([`parse_solid`](src/lib.rs));
2. **mesh** it with [manifold](https://github.com/elalish/manifold) — each `Solid` variant maps 1:1 to a
   manifold op, and the booleans produce a guaranteed-watertight mesh ([`mesh`](src/mesh.rs));
3. **write** the mesh — binary STL or binary glTF, chosen by the `-o` extension ([`stl`](src/stl.rs),
   [`gltf`](src/gltf.rs));
4. **report** the axis-aligned bounding box (min/max/size/center — the "does it fit the print bed?" answer,
   [`bounds`](src/bounds.rs)).

## Why a separate crate (not a `cdz` subcommand)

`cdz-cad` links `manifold-csg`, whose `-sys` crate builds the **C++ manifold3d library via cmake**. To keep
that heavy build out of the seed workspace and the corpus gate, this crate is **workspace-excluded** (its own
`[workspace]` table + a root `Cargo.toml` `exclude`), the same isolation `cdz-smith` uses. A dedicated
`cdz-cad` CI job (in `.github/workflows/checks.yml`) builds + tests it on demand. A future `cdz cad`
subcommand can simply shell out to this binary (the `cdz calc` precedent).

## Formats

| `-o` extension | format | notes |
|---|---|---|
| `.stl` | binary STL | universal printer format; lossy (no topology/units), a convenience. `--ascii` writes the human-readable ASCII variant instead |
| `.glb` | binary glTF 2.0 | design-primary, watertight-preserving, read by every web/3D viewer; zero extra deps |

3MF (a ZIP-of-XML format) is a later add — it would pull a zip + xml dependency.

## Build / test

```
cargo test    # from this directory (standalone workspace) — parser, mesh, stl, gltf, bounds + integration
```

Requires a C++ toolchain + cmake (for the manifold3d build). Verified on aarch64.

## Layout

- `src/lib.rs` — the `Solid`/`Vec3` CSG types + `parse_solid` (the s-expr parser)
- `src/mesh.rs` — `Solid` → manifold → neutral `Mesh` (interleaved f32 positions + u32 indices)
- `src/stl.rs` — `Mesh` → binary STL
- `src/gltf.rs` — `Mesh` → binary glTF (`.glb`)
- `src/bounds.rs` — the model's axis-aligned bounding box (via manifold)
- `src/main.rs` — the `cdz-cad` CLI (read → parse → mesh → write → report)
- `tests/examples_pipeline.rs` — end-to-end pins over the real `implementation/cad` example models
