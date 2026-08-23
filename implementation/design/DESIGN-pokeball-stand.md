# DESIGN — Pokéball stand (a printable CAD example + the /cad rough edges it surfaced)

Status: stand DELIVERED (operator-approved, 2026-08-23). Written by the `design-pokeball-stand` session.
This doc records (1) the delivered example, (2) three concrete `/cad` dogfooding findings — two fixed
in-session, one worked around — and (3) a follow-up FEATURE the operator asked for (settable, cascading
tessellation resolution, OpenSCAD-`$fn`-style) that should be built by a CAD vertical.

## 1. What it is

A 3D-printable display stand for an 80 mm printed Poké Ball: a **squat truncated cone (frustum) with a
spherical dimple in the top** so the ball nestles in a matching seat. Operator brief: "a cone with a
dimple in the top", squat & stable, ~Ø40 dimple, solid, export STL. It is an **example in the `/cad`
route's switcher**, NOT a member of the gated `cad` library (operator: "it should just be one of the
examples").

Delivered as a PARAMETRIC example (`guide/src/cad/examples.ts`, slug `pokeball-stand`) with five live
sliders — base Ø, top Ø, height, ball Ø, dimple depth. Defaults: **Ø90 base → Ø66 top, 40 mm tall, for
an Ø80 ball, 6 mm dimple** (a ~Ø42 seat mouth). Both surfaces (ml + sexpr) are verified to compile and
mesh against the real compiler + both mesh drivers.

### Geometry (why it's built this way)
- The CAD library has **no cone/frustum primitive** and `Solid.Scale` can't taper a cylinder, so the
  taper is a **lathe**: a trapezoid `Profile.PathProfile` (`path-start` → three `line-to`: base rim at
  `(base/2, 0)` → top rim `(top/2, height)` → back to the revolve axis) swept 360° with `Solid.Revolve`.
- The seat is `cut(frustum, move-z(height + (ball/2 − depth), ball(ball/2)))` — a sphere of the ball's
  own radius (so the ball seats flush), raised so it bites `depth` mm into the top face.
- The frustum stands up along **+Z**, base flat on `z = 0` (print bed). Verified mesh: 224 triangles at
  the default 32 segments, bounds `min[-45,-45,0] max[45,45,40]`.

### Export (STL)
`cdz compile <model> -o m.wasm && cdz run m.wasm [--host-response …] | cdz cad - -o stand.stl [--segments N]`
— resolution is a mesh-time flag (see §3), not baked. 32 → 224 tris; 128 → 1416 tris (print-quality).

## 2. Dogfooding findings

### F1 (fixed in-session) — `manifold.wasm` 404s in `vite` dev → `/cad` meshing dead
The `/cad` page meshes in-browser with `manifold-3d` (emscripten). Vite's dep-optimizer pre-bundles
manifold's JS into `.vite/deps/` but does NOT copy `manifold.wasm` beside it (esbuild can't follow the
runtime `locateFile` path), so `/node_modules/.vite/deps/manifold.wasm` falls through to the SPA shell
(HTML) and every `/cad` mesh dies with a WebAssembly magic-word error. **Fix:** add `manifold-3d` to
`optimizeDeps.exclude` in `guide/vite.config.ts` so it's served from `node_modules/manifold-3d/` where
`manifold.wasm` sits beside `manifold.js`. (This dev-mode path is not in CI — `check:visual` renders
`/cad` in a browser but isn't gated — so the bug was latent.)

### F2 (fixed in-session) — `MeshView` fixed camera can't frame a real-scale model
`guide/src/cad/MeshView.tsx` hard-coded the camera at `[4,3,5]` with no auto-fit. That suits single-
digit-mm demo models, but a real-scale part (the Ø90 stand spans `x,y ∈ [-45,45]`, `z ∈ [0,40]`)
ENGULFS the camera → you see back-face-culled interior walls → an "empty" preview with no error. **Fix:**
wrap the mesh in drei `<Bounds fit clip observe margin={1.2}>` (+ `makeDefault` on OrbitControls) so any
model auto-frames and re-fits when a slider re-meshes.

### F3 (worked around) — revolve height axis: model says Y, mesh says Z
`exact.cdz` documents `Solid.Revolve` as "about the y-axis" and its `bounding-box` scores the profile's
height on **Y**. But BOTH mesh drivers (native `cdz-cad` and browser `index.ts`, same manifold lib) stand
a revolve up on **+Z** (base flat on `z=0`). So the seat must be positioned with `move-z`, not `move-y`
(a `move-y` shaves a flank instead of dimpling the top). Net: the library's revolve `bounding-box` axis
disagrees with what the drivers actually mesh — misleading for anyone positioning relative to a revolve.
Owner call (v-cad): reconcile the `bounding-box` revolve arm with the drivers (height on Z, base at 0),
or document the axis explicitly.

## 3. Follow-up FEATURE (hand to a CAD vertical) — settable, cascading tessellation (`$fn`)

Operator ask: "set the number of segments and have it cascade like OpenSCAD … baking a global value
isn't going to work in all scenarios." Today tessellation of curved primitives (sphere/cylinder/circle)
and the revolve/path sweep is a **segment count**:
- native `cdz-cad`: `DEFAULT_SEGMENTS = 32`, overridable per-export with `--segments N` (min 3).
- browser `guide/src/cad/index.ts`: `const SEGMENTS = 32`, **hard-coded, no UI control** — this is the
  low-poly the operator sees; it's independent of the exported STL's resolution.

There is NO OpenSCAD-`$fn`-style, model-level, cascading resolution. The requested feature:
- A resolution value that **cascades** from a default down through the model to all curved leaves, with a
  **per-object / per-scope override** (the OpenSCAD `$fn` semantics — a global default a subtree can
  locally raise/lower for, e.g., a high-detail seat vs a coarse base).
- Surfaced consistently across: the CAD model (a way to carry the hint — a `Solid` node or an ambient
  parameter threaded through mesh), the native driver (`mesh_with_segments` already takes a global; needs
  per-node), the browser driver (`meshFromSolid` + `toManifold` thread a resolution, honor overrides), and
  the `/cad` UI (a quality control; and it should flow to STL export, not just preview).
- Open design question for the vertical: how to carry `$fn` in a PURE exact model — an ambient
  render-parameter threaded through the mesh walk (simplest, matches "cascade") plus an explicit
  per-object override node (e.g. `Solid.Detail(segments, child)`), vs. baking segments onto each curved
  primitive. Recommend the ambient-default + override-node shape (cascade with local override), keeping
  the model exact (segments is a MESH hint, never changes the exact geometry).

This is a v-cad-scale vertical (model + two drivers + UI + tests). Increment 1 (immediate value): a
preview segments control in `/cad` (slider → `meshFromSolid` resolution, re-mesh live). Increment 2: the
cascading `$fn` model hint + override, honored by both drivers. Increment 3: wire the resolution into
STL export from the UI.

## 4. Landing / hand-off notes
- The delivered change is a coherent unit: the `pokeball-stand` example (`examples.ts`) + the two `/cad`
  fixes it required (F1 `vite.config.ts`, F2 `MeshView.tsx`) + this doc. F1/F2 touch v-guide-infra files;
  flag to that owner. (Fleet was paused to platform-only when this landed — pr-sync/verticals stopped, so
  the MR + the §3 vertical brief queue for resume.)
