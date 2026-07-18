# DESIGN — Parametric snowflake + Assembly-as-code for Cadenza CAD

Status: **DRAFT / in progress** (v-cad, autonomous). Operator reviews async.
Scope: two operator-requested CAD showcases, driven autonomously on high-level requirements.
Companion to `DESIGN-cad-solid-modeling.md` (the base `Solid` CSG model).

## Context & mandate

Operator (verbatim, relayed via concierge):
- "a really cool example in the CAD page is my snowflake project. The algorithm is already
  deterministic but it would be awesome if we could just have a seed input that you feed it and it
  does a prng internally and builds out a unique snowflake! There's also some config that it would be
  great to parameterize."
- "I do not really care if you match the algorithm exactly … I just think it is a really powerful
  example of how code+cad can produce some really cool results!"
- "I do not have time to do design sessions today … I would actually be interested to see how far it
  can take it without my direction (other than high level requirements). It should definitely write up
  a design doc … so I can review async."

So this doc IS the tracking + review surface. Two features:
1. **Parametric snowflake** — a `@param seed` (Int64) drives an internal PRNG that builds a unique,
   deterministic snowflake; plus `@param` config sliders. Ships as a `/cad` showcase.
2. **Multi-model export + assembly-as-code** — multiple parts in one project, each exportable; an
   assembly that positions + combines parts in construct-style code (transforms / mates / a part tree).

Reference: the operator's real repo `camshaft/printing` (cloned at `../printing`), a workspace of Rust
parts built on `rsolid` (the CSG lib this Cadenza library mirrors). The snowflake lives at
`printing/snowflake/src/main.rs`.

## Part 1 — Parametric snowflake

### The reference algorithm (printing/snowflake/src/main.rs)
A recursive branching structure with 6-fold rotational symmetry:
- `Config { initial_branch_len, max_depth, max_children, max_width, min_width }`.
- `branch(rng, len, depth)`: builds a 2-D branch shape (a rounded bar = `square([len,width])` centred +
  a `circle(width/2)` at the tip). Then spawns `0..=max_children` child branches, each at a random
  offset along the bar, a random shorter length, a random rotation (30–170°), added AND mirrored across
  Y (bilateral symmetry). Recurses until `depth==max_depth` or `len<0.5`.
- `snowflake(seed)`: builds one branch, then rotates 6 copies at `360°*idx/6` and unions them
  (6-fold symmetry) → `linear_extrude(HEIGHT)` → centres in Z → adds/subtracts a magnet cylinder.
- Randomness is `rand_xoshiro`; the operator explicitly does NOT require matching it.

### What I keep / adapt
- **Do not match the exact RNG** (operator's call). Use our verified **MINSTD Lehmer LCG** (`prng.cdz`,
  landed separately): `seed : Int64 → state`, `next`, `roll(state,k) → [0,k)`, `between(state,lo,hi)`.
  Deterministic + seedable → the "unique per seed" property.
- **Keep** the recursive branch + 6-fold symmetry + bilateral-mirror structure — that's what makes it
  read as a snowflake and showcases code→CAD.
- **Config as `@param` sliders**: seed (Int64), branch-length, max-depth, max-children, width range.
  v-guide-infra single-mode auto-surfaces these; a fractional length is exact-Rational.

### CAPABILITY GAP #1 — the model has no `Rotate` (and no `Mirror`)
The snowflake needs **rotation about Z** (6-fold symmetry) and **mirror across Y** (bilateral
branches). The current `Solid` model (see `DESIGN-cad-solid-modeling.md`) has
`Empty/Cube/Sphere/Cylinder/Union/Difference/Intersection/Translate/Scale/ExtrudeLinear/Revolve` —
**no `Rotate`, no `Mirror`** — deliberately, because a general rotation is not exact over Rational
(it needs trig / irrational coordinates), and the model is exact-Rational.

**Decision (proposed): ADD `Rotate` and `Mirror` as model variants, following the EXACT precedent
`Revolve` already sets.** `Revolve` carries an exact Rational *angle in degrees* in the model and
defers the angular tessellation (the trig) to the f64 mesh-driver leaf — "the model carries the exact
degrees, no in-model trig." `Rotate` does the same:
- `Rotate(Vec3(a), Solid(a))` — an exact-Rational Euler-angle triple (degrees about x/y/z); the mesh
  driver applies the rotation matrix in f64 at the leaf (manifold `.rotate([x,y,z])`), exactly as it
  already does trig for `Revolve`/`Sphere`/`Cylinder` tessellation.
- `Mirror(Vec3(a), Solid(a))` — reflect across the plane with the given normal. Mirror IS exact over
  Rational (it's a sign flip, no trig), so it can be a true exact transform; but routing it through the
  mesh driver alongside Rotate keeps the drivers uniform. (Open: make Mirror exact-in-model vs
  mesh-leaf — leaning mesh-leaf for driver uniformity, since manifold has `mirror` directly.)

**Why this is not "inventing semantics":** it's the established `Revolve` pattern (exact angle in model,
f64 trig at the mesh edge) and it matches rsolid's own `.rotate`/`mirror`. `bounding-box` for a rotated
solid becomes approximate (the exact AABB of a rotated exact shape is not Rational) — handle by
bounding the rotated child's existing AABB corners conservatively, or documenting bbox as
best-effort for Rotate (bbox is used for layout, not geometry correctness).

**Impact:** touches `exact.cdz` (model + `SolidR` mirror + `lower` arms + bbox arms), both mesh drivers
(native `cdz-cad` + browser `guide/src/cad/index.ts`), and the render grammar (PING v-guide-infra per
the grammar-change rule). This is the critical-path enabler for BOTH the snowflake and assemblies
(mating parts needs rotation). **Build `Rotate`/`Mirror` first, then the snowflake on top.**

### Snowflake build plan (incremental, each gated)
1. Add `Rotate` (+ `Mirror`) to the model + both drivers + grammar ping. (enabler)
2. `snowflake.cdz`: the recursive branch builder over the LCG stream + 6-fold symmetry, prelude-only,
   returning `lower(...)`. Non-parametric core pinned by @tests (fixed seed → stable structure).
3. `showcase-snowflake.cdz`: the `@param seed + config` entry (`main = host Param in …`), sliders.
4. Coordinate sliders + (optional) a seed "randomize" button with v-guide-infra.

## Part 2 — Multi-model export + assembly-as-code

**High-level requirements (my interpretation, operator to confirm async):**
- A project can define **multiple named parts**, each independently exportable (STL/3MF/AMF).
- An **assembly** positions + combines parts in ordinary Cadenza code: transforms (translate / rotate /
  mirror / scale), "mates" (place part B relative to a feature of part A), and a **part tree**
  (sub-assemblies compose).
- Study `printing`'s part-list patterns for what real assemblies need. *(Detailed survey in progress —
  Explore agent reading family-room, clock-*, stick-up-cam, weather-station, can-attachment, etc. This
  section fills in once that lands.)*

### Design sketch (pre-survey, to refine)
- **A part is just a `Solid`** (or a named `def part-name() = lower(...)`). No new type needed for
  "a part."
- **An assembly is a `Solid` too** — built by `Union`-ing transformed parts. "Mating" = expressing part
  B's placement as `Translate`/`Rotate` relative to part A's known features (dimensions the parametric
  model already carries exactly). Construct-style: `assembly = fuse(base, move-z(base-height,
  rotate-z(90, lid)))`.
- **A part tree** falls out of nesting: an assembly is a `Solid`, so a bigger assembly unions
  sub-assemblies. Optional: a lightweight `Assembly` record `{ name, solid, children }` for
  export/BOM metadata, if the survey shows real assemblies want named-part export + a bill of materials.
- **Multi-model export**: the reference `main()` iterates a list of `(name, seed, solid)` and exports
  each. Cadenza analog: a project exposes several `@export`-ed part defs; `/cad` (v-guide-infra) offers
  a per-part export picker + the STL/3MF download button (writers already exist: `stl.ts`,
  `threemf.ts`). CLI: `cdz run` per part, or a manifest of parts.

### CAPABILITY GAP #2 — mating needs `Rotate` (shared with Part 1) + likely nothing else
Real assemblies rotate parts to mate them → same `Rotate` enabler as the snowflake. Beyond that,
positioning is `Translate`/`Scale`/`Union` which the model already has. A named-part/BOM record is
additive metadata, not a core-model change. *(Confirm against the survey.)*

## Open questions (for async operator review — non-blocking; I proceed on my best call)
1. `Rotate`/`Mirror` as mesh-leaf f64 transforms (matching `Revolve`) — OK? (My plan: yes.)
2. Assembly metadata: is a named-part + BOM record wanted, or is "an assembly is just a `Solid`"
   enough for the showcase? (My plan: start with Solid-only; add a record only if the survey shows a
   real need.)
3. Snowflake magnet feature (the reference adds/subtracts a magnet mount) — include, or keep the
   showcase pure-geometry (no printer-specific mount)? (My plan: pure geometry for the showcase, note
   the magnet as an optional config.)

## Progress log
- 2026-07-18: doc created. PRNG solved + built (`prng.cdz`, MINSTD LCG, MR sent) — Part 1's capability
  #2 closed. Studied `printing/snowflake` — algorithm understood. Identified capability gap #1
  (`Rotate`/`Mirror`) as the shared critical-path enabler. Assembly survey (Explore) in progress.
