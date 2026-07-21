# DESIGN — Cadenza CAD: a solid-modeling library + manifold-backed render, in code

*2026-07-15. Operator directive (GH #400, verbatim): "I want to get a new vertical dedicated to building
out a new tool using cadenza. I want to build a openscad like environment where you describe models in
code and have it rendered. We should use the manifold library. We should use the existing ide
infrastructure in the browser (but I don't want it to be limited to the browser, similar to the
calculator). You can look at https://github.com/camshaft/rsolid and my printing repo for kind of what
I'm after … port rsolid to cadenza and make a really nice interface for it. I don't really care about the
openscad backend - I think manifold is a lot faster route. And I'm pretty sure it compiles to wasm too."*

> **STATUS: ✅ SHIPPED & LIVE (updated 2026-07-21). The vertical delivered end-to-end; both operator
> follow-on features (assembly-as-code + parametric snowflake) are live in the `/cad` picker.** The
> reconciliation below is the current state; the original 2026-07-15 design pass (from §0 down) is retained
> verbatim as the record — but note it describes a **Float64** geometry model that was SUPERSEDED by the
> operator's exact-Rational/Qty redirect (see the ⚠ below).
>
> ### ✅ What shipped (read this first — supersedes the older progress notes)
> - **The model went EXACT-RATIONAL, not Float64.** The operator redirected mid-build ("it should all be
>   RATIONALS everywhere … strong typing about units"): `Vec3`/`Solid` are **generic over the coordinate
>   type**, the CAD model is exact `Rational` (units-carrying `Qty(Rational, meter)` in `units-model.cdz`),
>   and `f64` appears ONLY at the manifold FFI leaf. So §§ below that say "Numerics are Float64" / "Vec3 =
>   record of Float64" are the OLD plan — superseded. (Companion docs: `DESIGN-cad-units-everywhere.md` P1,
>   `DESIGN-cad-profiles-extrude-paths.md`.)
> - **`bounding-box` SHIPPED** (exact, in `exact.cdz`) — the Float64-compare gap that deferred it is moot
>   over Rational (total order); the recursive fold is exact. (The one-time O(2^n) ANF match-binder-reuse
>   miscompile it later hit was fixed by v-compiler-perf.)
> - **G1 model** — `Empty/Cube/Sphere/Cylinder` + booleans + `Translate/Scale` + **`Rotate`/`Mirror`**
>   (added as the snowflake/assembly enabler) + `ExtrudeLinear`/`Revolve` + `Path`/`Profile` (splines) +
>   `simplify-r` + exact `bounding-box`. 119 `@test`, gated by the `cad-tests` CI job.
> - **G2 native driver** (`cdz-cad`, workspace-excluded) — parse SolidR s-expr → manifold mesh → STL +
>   binary glTF → bounding box. 54 tests. Manifold is a per-surface DRIVER (B1), not a Cadenza peer.
> - **G3 browser `/cad` — SHIPPED & LIVE** (the scope call resolved to in-guide, lazy-route). Compiles a
>   BARE model buffer against the **preloaded** CAD library (P5, `compile_with_preloaded`) — the reader edits
>   only their model. Renders via `manifold-3d` + three.js; the picker carries the primitive showcases +
>   both operator features (assembly-l-bracket, assembly-parametric-bracket, parametric-snowflake).
> - **Both operator follow-on features delivered + operator-confirmed:** assembly-as-code (plain + parametric
>   L-bracket) and the seed→unique parametric snowflake (built inline from primitives).
> - **D1/D2** (§3) shipped on their chosen defaults, borne out.
>
> The original design pass follows, verbatim (Float64-era — see the redirect note above).

This design leans on one structural finding, exactly the way the calculator design did: **almost none of
this is new language work.** A CSG model is *Cadenza data* — a recursive sum (the CSG tree) over records
(the parameters). Building, composing, and transforming that tree is ordinary Cadenza the language already
compiles and runs today (sums, records, recursion, floats, the multi-surface run pipeline). The genuinely
new work is narrow and lives at the **edges**: (1) a mesh has to come out the other end, and manifold is
the geometry kernel that does it; (2) a *render surface* to see it. rsolid's design (a tree of operations
that renders to a backend) ports almost verbatim — we swap its OpenSCAD-text backend for a manifold-mesh
backend, which is the operator's whole point.

---

## §0 — Vision, and what it showcases

Describe a parametric solid in Cadenza and *see it*:

```
-- a plate with two bolt holes, fully parametric
def (plate (: w Float64) (: d Float64) (: t Float64) (: r Float64))
  (difference
    (cube w d t)
    (union
      (translate (v3 (* w 0.25) (* d 0.5) 0.0) (cylinder t r))
      (translate (v3 (* w 0.75) (* d 0.5) 0.0) (cylinder t r))))
```

- You write the model as **code** — parametric, composable, recursive (a gear = fold a tooth around a
  circle; a bracket = a `union` over a list of ribs). This is the marquee: a CSG tree is a *natural*
  Cadenza data structure, so the language shows off exactly what it's good at.
- It **renders** — the CSG tree compiles down to a manifold mesh you can see (a live 3D preview in the
  guide) and export (a 3MF/glTF file, or STL for a printer).
- It runs in **more than one place off one core**, the calculator's proven multi-surface model: a native
  `cdz cad` (mesh → file) and a guide `/cad` page (live in-browser preview). Not browser-limited.

This is the calculator lesson applied to geometry: the *language and its run pipeline already exist*; the
tool is a **library + a backend + a render surface** on top of them.

---

## §1 — The one finding that shapes everything: a CSG tree is Cadenza data

rsolid (the thing to port) is, structurally, a **tree of operations that renders to a backend**: a `Scad`
trait with `to_scad()`, an `Object`/`Operator` split (leaves = primitives, nodes = boolean ops +
transforms), and `export()` walks the tree emitting OpenSCAD text that the `openscad` binary then meshes.
The operator wants to keep the *tree* and *replace the backend* (openscad-text → manifold-mesh).

In Cadenza that tree is one recursive sum — **no language feature is missing to express it**:

```
type Vec3   = (record (x Float64) (y Float64) (z Float64))

type Solid  =
  ( sum
    -- leaves: primitives (parameters in records)
    (Cube      (record (size Vec3)))
    (Sphere    (record (r Float64)))
    (Cylinder  (record (h Float64) (r Float64)))
    -- nodes: boolean operations over child solids (recursion)
    (Union        (record (a Solid) (b Solid)))
    (Difference   (record (a Solid) (b Solid)))
    (Intersection (record (a Solid) (b Solid)))
    -- nodes: transforms wrapping a child solid
    (Translate (record (by Vec3)   (of Solid)))
    (Rotate    (record (deg Vec3)  (of Solid)))
    (Scale     (record (by Vec3)   (of Solid))) )
```

Everything above is data the compiler handles today — sums, records, recursion, floats. `cube`, `union`,
`translate`, … are ordinary constructor-helper `def`s. `n`-ary `union`/`difference` over a *list* of
solids is a `fold`. A gear/thread/bracket from the operator's "printing repo" is a recursive Cadenza
function producing a `Solid`. **This whole layer — the model DSL — needs zero compiler or runtime change**
(it's the direct analogue of the calculator being "a state layer over an existing primitive").

**Consequence, and the honest scope call:** the "port rsolid to Cadenza" ask is *mostly a Cadenza library*,
plus a numerics helper layer (Vec3 / 4×4 affine matrix math for `transform`, all Float64 — the language
has floats and records, so this is library code too). The two things that are genuinely NOT library code
are the **mesh backend** (§2) and the **render surfaces** (§4). Scope the vertical around those two edges;
the DSL is the cheap, high-value middle.

---

## §2 — The mesh backend: manifold, and the honest binding finding

This is where "render it" lives, and where the one real research question is. The CSG tree has to become a
triangle mesh; manifold is the kernel that evaluates the booleans into a guaranteed-watertight mesh.

**Confirmed facts about manifold (elalish/manifold):**
- C++ core; **compiles to WebAssembly** — the npm package `manifold-3d` is "built via WASM" (confirms the
  operator's "pretty sure it compiles to wasm"). Its API is *OpenSCAD-inspired*: primitive constructors,
  a guaranteed-manifold boolean (the CAD kernel), transforms, and mesh access.
- A **Rust crate exists** — `manifold-csg` on crates.io (an external binding to the C++/wasm core) — the
  path for a **native** surface.
- Export: manifold discourages STL (lossy); recommends **3MF** and **glTF/GLB** (`EXT_mesh_manifold`). We
  support 3MF/glTF as primary and STL as a convenience for printers.

**The honest binding finding (this is the key research result, flag it up front).** `manifold-3d` is an
**Emscripten-built WASM module with a JS glue API** — it is *not* a WebAssembly **Component** with a WIT
interface. So it does **not** drop into Cadenza's cross-component peer-interop story
(`DESIGN-cross-component-interop-rcdzc.md`) the way the assign's optimistic framing hoped ("fits Cadenza's
cross-component/peer interop story"). A Cadenza `(bind …)` peer needs a component exporting a WIT interface
over the shared value-heap runtime; manifold exposes neither. **Do not spend the vertical trying to make
manifold a Cadenza peer** — that's a componentization project unto itself, orthogonal to this feature.

Instead, manifold binds the way the calculator's *effects* already work and the way rsolid *already*
structures its backend — as an **external geometry service the Cadenza program calls, once, at the leaf**:
the Cadenza side builds and owns the CSG *tree* (pure data); a thin **mesh driver** on each surface walks
that tree and calls manifold's native ops to produce the mesh. Two clean shapes, one per surface, sharing
the tree:

- **Native (`cdz cad`):** the driver is Rust, links the `manifold-csg` crate, walks the CSG tree (received
  from the compiled Cadenza program as a value it reads — a runtime handle / rendered value) and calls
  manifold to mesh + export. This is the rsolid `export()` shape with manifold swapped for openscad-text.
- **Browser (`/cad` page):** the driver is TS, imports `manifold-3d` (the WASM npm pkg), walks the same
  tree (crossed out of the run worker) and calls manifold-wasm to mesh, then hands the mesh to a three.js
  preview.

**Two ways for the tree to reach the driver — pick the seam (⚑ decision D2, §3):**
- **(B1) Render-the-tree-as-data (recommended default).** The Cadenza program's entry returns the `Solid`
  value; the surface's run pipeline already renders/crosses a compound value (the calculator's
  `value-bytes` path in the browser, `cdz-run`'s `Outcome::Value` natively). The driver receives the CSG
  tree as a *plain data value* (a rendered/decoded `Solid`) and interprets it. **No compiler change** — it
  reuses the exact compound-value-crossing the calculator uses. The mesh kernel stays entirely
  host/driver-side; Cadenza never calls manifold, it just *produces the tree*. This is the cleanest fit
  and the direct analogue of rsolid's `to_scad()` → external renderer.
- **(B2) Manifold-as-an-effect.** Model manifold as a Cadenza `(effect Manifold (op union …) …)` the
  program performs, host-bound per surface (the `(host (Manifold) …)` shape, `capabilities-and-effects.md`).
  More "in the language," but it makes every boolean a host round-trip and needs a host-effect handler per
  op on each surface — heavier, and it buys nothing over B1 for a batch mesh-once workload. Keep as a
  documented alternative; it becomes attractive only if we later want *incremental* re-meshing.

**Recommendation: B1.** Cadenza owns the tree (pure, testable, gradeable as data); the mesh kernel is a
per-surface driver that consumes the tree. This maximizes the "showcase Cadenza's data modeling" value and
minimizes compiler surface — the only compiler-adjacent question (B2's effect) is explicitly *not* on the
path.

---

## §3 — Decisions

1. **The model DSL is a Cadenza library** (§1) — a recursive `Solid` sum + Vec3/affine-matrix helpers +
   constructor/combinator `def`s. Zero compiler/runtime change; this is the bulk of "port rsolid."

2. **Keep rsolid's tree; replace its backend.** Port rsolid's `Object`/`Operator` tree structure to the
   `Solid` sum; replace the `to_scad()`/openscad-binary backend with a **manifold mesh backend**. We do
   NOT support the openscad backend (operator: "I don't really care about the openscad backend").

3. **manifold binds as a per-surface mesh driver, NOT a Cadenza peer** (§2). `manifold-3d` is Emscripten
   WASM + JS glue, not a WIT component; `manifold-csg` is the native Rust crate. The Cadenza program
   produces the CSG *tree as data* (B1); a native (Rust/`manifold-csg`) and a browser (TS/`manifold-3d`)
   driver each walk the tree and mesh it. This is the honest correction to the assign's "manifold as a
   Cadenza peer" framing — surfaced as a finding.

4. **Multi-surface, one core** (calculator model): the `Solid` library is the shared core; a native
   `cdz cad model.cdz -o out.3mf` and a guide `/cad` live-preview page are the two surfaces. Not
   browser-limited (operator directive).

5. **Render targets:** 3MF + glTF primary (manifold's recommended, watertight-preserving), STL as a
   printer convenience. Live preview = three.js in the guide (`manifold-3d` → mesh → `BufferGeometry`).

6. **Numerics are Float64** for geometry (meshes are float; matches manifold). Vec3 + 4×4 affine matrix
   is library code (records + Float64 arithmetic). Exact rationals/units are NOT needed for the mesh path
   (a future "dimensioned models" idea could layer `Qty` on top — noted, out of scope).

**⚑ Open decisions flagged to the operator (concierge `ask`), each with a chosen default so build proceeds:**

- **⚑ D1 — surface syntax: s-expr `Solid` DSL now, or ML sugar?** The examples above use the s-expr
  surface. A "really nice interface" (operator's words) probably wants the ML surface
  (`difference (cube w d t) (union …)`) and maybe operator sugar (`a - b` for difference, `a + b` for
  union — the OpenSCAD/`rsolid` feel). **Default: build the library + drivers on the plain s-expr/ML
  applicative surface first (no new operators); evaluate `+`/`-`/`*` overloading for solids as a later
  increment (it's type-directed dispatch, a library-level nicety, not a new language form).** Asking the
  operator whether the sugar is a must-have for the first cut.

- **⚑ D2 — the tree→driver seam: render-as-data (B1) or manifold-as-effect (B2)?** §2. **Default B1**
  (render the tree as data, driver meshes it). Asking to confirm they're happy with Cadenza *producing*
  the model rather than *calling* manifold from inside the language (B1 is simpler and showcases data
  modeling; B2 is "more in the language" but heavier and buys incremental re-meshing we don't need yet).

Both asks ship with a default; the vertical starts on the defaults and only adjusts if the operator
redirects.

---

## §4 — The surfaces (calculator's multi-surface model, applied)

### 4a. Native — `cdz cad model.cdz -o out.3mf` (recommended first surface after the library)

A `cdz cad` subcommand (the `cdz calc` precedent): compile the model program (`rcdzc::compile`), run it
(`cdz-run`) to obtain the `Solid` tree as a value, hand the tree to the **native mesh driver** (Rust,
`manifold-csg`) which walks it → manifold mesh → write 3MF/glTF/STL. `--preview` could pop a window later;
the first cut is file-out (scriptable, testable, no GUI dependency — the `--once` analogue). This is the
reference driver everything else mirrors.

### 4b. Browser — a guide `/cad` route (live preview)

The guide already has a lazy full-screen `/calculator` route reusing the run worker + jco path
(`guide/src/main.tsx`, `guide/src/calculator/`). `/cad` is the same shape: an editor (reuse the
playground's), a run that produces the `Solid` tree (reuse `replEval`/the run worker crossing a compound
value), a **browser mesh driver** (TS, `manifold-3d` npm) that walks the tree → mesh, and a **three.js
canvas** for live preview (orbit, wireframe toggle, export button → 3MF/glTF/STL download). New route in
`main.tsx`, lazy-loaded like the calculator; deploys on the existing Pages workflow. New deps: `three` (+
`@react-three/fiber` for the React canvas) and `manifold-3d`. Verify in a real browser (the guide's
Playwright recipe): the `plate` example renders a plate with two holes; changing a parameter re-meshes.

### 4c. Later surfaces (noted, not first-cut)

A native preview window (Tauri/winit + wgpu), a Raycast-less "open model" app — the calculator's C5-style
packaging. Out of scope for the first vertical; the file-out native path + the browser preview are the two
that prove "describe in code + see it."

---

## §5 — Increment plan (each a landable slice; gate 0-fail per step)

Ordered library-first, so a *usable, testable* CSG model layer exists before any mesh/render dependency —
exactly the calculator's engine-first ordering.

- **G0 — this design doc + a `cad/` library skeleton.** Land this doc. Create the model library location
  (a Cadenza source module — decide `implementation/cad/` or a guide-adjacent location with the
  vertical), plus the two ⚑ `ask`s to the concierge. No behavior yet.

- **G1 — the `Solid` model + primitives + boolean ops + transforms (pure Cadenza, no mesh).** The `Solid`
  sum (§1), `Vec3` + 4×4 affine-matrix helpers (Float64), constructor `def`s (`cube`/`sphere`/`cylinder`),
  boolean combinators (`union`/`difference`/`intersection`, binary + `n`-ary via `fold`), transforms
  (`translate`/`rotate`/`scale`, composing matrices). **Gate: pure-data unit tests** — build a tree,
  assert its shape; a `bounding-box`/`count-nodes` fold to prove recursion runs end-to-end under wasmtime.
  This is the "port rsolid's tree" milestone and needs **zero** compiler change — it's the high-value core.
  ⚠ This exercises recursion + sums + records + floats hard; **report (don't work around)** anything the
  language can't express cleanly (per the assign) — a finding for the relevant vertical.

- **G2 — the native mesh driver + `cdz cad` (file-out).** A Rust mesh driver linking `manifold-csg`:
  receive the `Solid` tree (as the compiled program's returned value, B1), walk it → manifold mesh →
  write 3MF/glTF/STL. New `cdz cad model.cdz -o out.3mf` subcommand (the `cdz calc` shape). **Gate:** an
  integration test compiles+runs the `plate` example, meshes it, and asserts the output mesh is
  non-empty + watertight (manifold guarantees this) + has the expected genus (a plate with 2 holes → 2
  through-holes). ⚠ New native dep `manifold-csg` — confirm it builds on this box's aarch64 toolchain
  (a build-time finding if it needs the C++ toolchain / a vendored libmanifold).

- **G3 — the browser `/cad` route + live three.js preview.** A guide route mirroring `/calculator`:
  editor → run (tree as data) → TS mesh driver (`manifold-3d`) → three.js canvas (orbit + wireframe +
  export-download). New deps `three`/`@react-three/fiber`/`manifold-3d`; lazy route in `main.tsx`;
  existing Pages deploy. **Gate:** Playwright — the `plate` example renders, a parameter edit re-meshes,
  export downloads a 3MF. Reuses the run worker + compound-value crossing verbatim (no compiler change).

- **G4 — worked examples from the "printing repo" + a `/cad`-first example gallery.** Port 2–3 real models
  from the operator's printing repo (a bracket, a gear, a parametric box) as Cadenza `Solid` programs —
  the "use those as examples of how to build things up" ask. Doubles as a corpus of real-world Cadenza
  stress programs (recursion, list-folds, float math). **Gate:** each example compiles, runs, and meshes
  non-empty on both surfaces.

- **G5 — the "really nice interface" polish (⚑ D1-dependent).** If the operator wants it (D1): solid
  operator sugar (`a + b`/`a - b`/`a * b` = union/difference/intersection via type-directed dispatch — a
  library/dispatch nicety, evaluated as a *separate* increment because it may touch operator resolution),
  parameter sliders in `/cad` (live-tweak a model's numeric params, re-mesh on change — the parametric-CAD
  feel), a model gallery. Scope firmed once D1 comes back.

**Ordering rationale:** G1 gives a complete, tested CSG *model* layer with zero external dependency — the
part that best showcases Cadenza and carries the most risk of finding a language gap. G2 is the first
*render* (native, scriptable, the reference driver). G3 is the compelling "see it live in the browser"
demo. G4 proves it on the operator's real models. G5 is the interface polish, gated on the D1 answer.

---

## §6 — Reusable vs. net-new (at a glance)

| Piece | Status |
|---|---|
| Recursive sum + records + Float64 to model a CSG tree | ✅ language supports today — the model DSL is ordinary Cadenza |
| Compile + run a program, cross a compound value out (native + browser) | ✅ exists — `cdz-run` `Outcome::Value`, calculator's `value-bytes`/run-worker path |
| Guide full-screen lazy route + Pages deploy + run worker | ✅ exists — `/calculator` precedent (`main.tsx`, `guide/src/calculator/`, `runner/`) |
| `cdz` subcommand pattern (`cdz calc`) | ✅ exists — model for `cdz cad` |
| The `Solid` model library (sum + helpers + combinators + transforms) | 🔨 G1 — net-new Cadenza library (no compiler change) |
| Native mesh driver (`manifold-csg`) + `cdz cad` file-out | 🔨 G2 — net-new Rust driver + subcommand + native dep |
| Browser mesh driver (`manifold-3d`) + three.js preview + `/cad` route | 🔨 G3 — net-new TS driver + route + npm deps |
| Worked examples (printing repo) | 🔨 G4 — net-new Cadenza programs (also a stress corpus) |
| Solid operator sugar + parameter sliders | 🔨 G5 — net-new, ⚑ D1-gated (dispatch + UI) |
| manifold as a **Cadenza WIT peer** | ❌ not viable — manifold-3d is Emscripten WASM + JS glue, not a component; bind as a per-surface driver instead (§2) |
| openscad-text backend | ❌ dropped — operator prefers manifold |

---

## §7 — Risks / watch-items

- **manifold is NOT a Cadenza peer.** `manifold-3d` is Emscripten WASM with a JS glue API, not a WIT
  component over the shared value-heap runtime — it can't `(bind …)` as a peer (§2). Bind it as a
  per-surface mesh driver (native `manifold-csg`, browser `manifold-3d`) consuming the CSG tree as data.
  Correcting the assign's "fits the peer-interop story" framing is finding #1.
- **The value the driver consumes is the CSG tree, produced by Cadenza — Cadenza does NOT call manifold**
  (B1, the recommended seam). Keep the mesh kernel host/driver-side; the language just produces data.
- **Report language gaps, don't work around them** (assign directive). G1/G4 stress recursion + sums +
  records + floats; if the model can't be expressed cleanly (a missing fold shape, a float-precision
  issue, a records ergonomics gap), file it as a finding to the owning vertical — this is a real stress
  test, the calculator's role for numerics.
- **Native dep build (`manifold-csg`).** May need a C++ toolchain / vendored libmanifold; confirm it
  builds on aarch64 in G2 before committing to the native driver shape (a build-time finding, possibly a
  concierge `ask` if it needs infra).
- **Browser bundle weight.** `three` + `manifold-3d` are heavy; keep `/cad` lazy-loaded (the calculator
  route already establishes lazy full-screen routes) so the guide's first paint stays light.
- **Float determinism across surfaces.** A mesh built native (`manifold-csg`) vs browser (`manifold-3d`)
  may differ in the last ULP; don't assert byte-identical meshes across surfaces — assert *topological*
  properties (non-empty, watertight, genus/hole-count) in gates.
- **STL is lossy** (manifold's own guidance); prefer 3MF/glTF, offer STL only as a printer convenience.
- **Don't reimplement the tree-walk per surface's semantics.** The `Solid` tree is the single source of
  truth; each driver only *evaluates* it to a mesh — the tree's meaning (what `difference` means) lives in
  the Cadenza library + the manifold ops, never re-specified in TS vs Rust.

---

## §8 — Summary

An OpenSCAD-like CAD tool in Cadenza is, at its heart, **a Cadenza library**: a recursive `Solid` sum over
parameter records, with Vec3/affine-matrix helpers and constructor/boolean/transform combinators — a CSG
*tree as ordinary Cadenza data*, which is exactly what the language's sums + records + recursion are for
(the port of rsolid's operation-tree). The genuinely new work is at two edges: a **manifold mesh backend**
(the geometry kernel that turns the tree into a watertight mesh) and **render surfaces** (a native
`cdz cad` file-out driver via the `manifold-csg` crate, and a guide `/cad` live three.js preview via the
`manifold-3d` wasm pkg) — the calculator's proven multi-surface, one-core model. The one honest correction
to the intake: **manifold is not a Cadenza WIT peer** (it's Emscripten WASM + JS glue), so it binds as a
per-surface driver that consumes the CSG tree *as data* (B1) — Cadenza produces the model, the driver
meshes it. Two decisions are flagged to the operator (surface sugar D1, tree→driver seam D2) each with a
build-unblocking default. Build library-first (G1, zero compiler change, maximal showcase), then native
render (G2), then the live browser preview (G3), then the operator's real models (G4), then interface
polish (G5). The DSL needs no compiler or runtime change; the mesh backend and the render surfaces are the
real, well-bounded new work — and the whole thing is a strong real-world stress test of Cadenza's data
modeling, the way the calculator was for its numerics.
