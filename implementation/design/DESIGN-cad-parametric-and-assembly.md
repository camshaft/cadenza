# DESIGN — Parametric snowflake + Assembly-as-code for Cadenza CAD

Status: **✅ SHIPPED + OPERATOR-CONFIRMED (both features delivered, live in the `/cad` picker).** This doc is
now a RECORD, not an in-progress draft; the reconciliation box just below records what shipped. Retained as
the design/tracking surface the operator reviewed async.
Scope: two operator-requested CAD showcases, driven autonomously on high-level requirements.
Companion to `DESIGN-cad-solid-modeling.md` (the base `Solid` CSG model).

## ✅ What shipped (reconciliation — read this first)

Both operator-requested features landed on trunk, are live in the `/cad` picker, and were operator-confirmed
(the snowflake after a design-directive + blank-render round the operator rejected and I re-architected).

- **Parametric snowflake** — `showcase-snowflake.cdz`, picker slug `parametric-snowflake`. The recursive
  branch + 6-fold symmetry + bilateral mirror + MINSTD-LCG PRNG are built **inline from visible primitives**
  (box/ball/fuse/move-x/rotate-z/mirror-x) — NOT an opaque `snowflake()` library fn (the operator rejected
  a hidden impl: "the whole point is you can create a snowflake from simple primitives"). `@!param` seed
  (Int64) + arm-length + depth sliders; seed→unique, deterministic, browser-verified **49930 tris visible**,
  drag-seed re-meshes a different snowflake. Uses threaded `prng.cdz` (the Prng-EFFECT swap is a ready
  future internal upgrade, not required).
- **Assembly-as-code** — `showcase-assembly.cdz` (plain L-bracket) + `showcase-assembly-parametric.cdz`
  (5 `@!param` dimension sliders), picker slugs `assembly-l-bracket` + `assembly-parametric-bracket`. An
  assembly IS just a `Solid` (named part defs mated via origin-relative Translate/Rotate/Mirror over shared
  dims) — confirmed against the operator's real parts (no formal mate/part-tree system needed). Rotate/Mirror
  were the enabler this drove (added as mesh-leaf transforms, Revolve precedent).
- **The autonomous run surfaced + got fixed ~6 real compiler/runtime/tooling bugs** (all non-CAD, each
  routed to its owner): bbox O(2^n) ANF match-binder-reuse, @param-comment CDZ0201, jco whole-kebab
  camelCase, 818759e9 value-heap runtime OOB, Int64-@param accessor, empty-Solid mesh-annihilation.
- The Rotate/Mirror decision (mesh-leaf f64, model carries exact Rational angle) noted in Part 3 below
  shipped as designed; the sound L1 rotate-bbox radius is the exact-model bound.

The original draft (algorithm study, decisions, assembly sketch) is retained verbatim below as the record.

---


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
### Survey findings (from `printing`'s real parts — clock-*, family-room, stick-up-cam, gingerbread, goggle-clip, weather-station, can-attachment)
The survey **confirmed the sketch below** and sharpened it. Key facts about how his real assemblies
actually work:
- **No formal mate/constraint system, no explicit part-tree data structure.** "Assembly" = ordinary
  function composition: each component is a `fn foo() -> Object`, and `main` unions them
  (`face() + dial() + rim() + posts()`). The **call graph of the component functions IS the part tree.**
- **Placement is origin-relative transform chains**, not B-relative-to-A frames. `part >> right(x) >>
  back(y) >> up(z)`, with shared `const`s (BASE_R, WIDTH…) so parts line up by construction.
- **Mirror is the primary symmetry/mating tool**: `shape += &shape >> mirror([1,0,0])` (bilateral
  parts) is pervasive.
- **Reference/background parts** (`.bg()`) show hardware being mounted-to; the real part is carved to
  fit (`c -= power_supply() >> scale(1.02)`). The closest thing to a "mate against existing hardware."
- **The one genuine B-relative-to-A mate** (stick-up-cam): `mount >> rotate_y(90) >> fwd(CLIP_W*0.5 +
  MOUNT_D*0.5 + 1.0)` — a rotation plus a dimension-arithmetic offset. That's the manual mate idiom.
- **Rotational arrays via loops**: `>> rotate([0,0, i/total*360.0])` — exactly the snowflake's 6-fold.
- **Only snowflake is `Config`-driven** (a struct of knobs); other parts parameterize via top-level
  `const`s + fn args.

### Design (confirmed)
- **A part is just a `Solid`** (a named `def part-name() = lower(...)`). No new type needed. The part
  tree is the call graph of these defs — exactly rsolid's model.
- **An assembly is a `Solid` too** — `Union` of transformed parts, positioned origin-relative with the
  ergonomic helpers. Construct-style: `assembly = fuse(base, move-z(h, rotate-z(90, lid)))`. "Mating" =
  a `Rotate` + a `Translate` whose offset is dimension arithmetic over the parts' known exact
  dimensions (the parametric model already carries them exactly). No constraint solver — matches how
  his parts actually mate.
- **Bilateral symmetry** = `fuse(s, mirror-x(s))`; **rotational arrays** = fold `Union` over
  `rotate-z(i/n * 360, s)`. Both fall out of `Rotate`/`Mirror` + `Union`.
- **Multi-model export**: a project exposes several part defs; `/cad` (v-guide-infra) offers a per-part
  picker + the STL/3MF download button (writers already exist: `stl.ts`, `threemf.ts`). CLI: `cdz run`
  per part. A named-part collection (rsolid's `Vec<(name, solid)>`) maps to several exported defs.
- **Assembly metadata record** — DEFER. The survey shows no real need for a BOM/`Assembly {name,
  children}` type; "an assembly is just a `Solid`" is sufficient for the showcase. Add only if a
  concrete need appears.

### CAPABILITY GAP #2 — mating needs `Rotate`/`Mirror` (shared with Part 1); nothing else core
Real assemblies rotate + mirror parts to mate them → the **same `Rotate`/`Mirror` enabler** as the
snowflake, and nothing else in the core model (positioning is `Translate`/`Scale`/`Union`, already
present). Confirmed by the survey.

**Exactness sharpening (important survey insight):** most *mating* rotations in his parts are
**90°/180°/±90° and axis-aligned mirrors — which ARE exactly Rational** (a 90° rotation permutes/negates
coordinates; an axis mirror is a sign flip). Only *decorative* rotations (arrays, the snowflake's 60°
and 30–170° children) need arbitrary angles = f64 at the mesh leaf. So the `Rotate`/`Mirror` design
splits cleanly:
- **`Mirror(normal, Solid)`** across an axis-aligned plane → **exact in-model** (sign flip on one
  coordinate; bbox stays exact). Cheap, exact, do it properly.
- **`Rotate(euler-degrees, Solid)`** → **mesh-leaf f64** (the `Revolve` precedent), because a general
  angle isn't Rational. A future refinement could keep 90°-multiples exact in-model (exact bbox, exact
  coords) and only defer arbitrary angles — worth doing since assemblies lean on 90° mates, but v1 can
  treat all `Rotate` as mesh-leaf and note the 90°-exact optimization as follow-up.

### Deferred capabilities the survey surfaced (NOT needed for snowflake or basic assembly; logged)
Real reproduction of his full part catalog would also want: **cone / tapered cylinder** (`cone(h,r1,r2)`
— dishes/rims), a **2D sketch subsystem** (Square/Circle/Polygon — the DSL is sketch-then-extrude; we
have `ExtrudeLinear`/`Revolve` + `Profile` but a fuller 2D primitive set would help), **Minkowski sum**
(fillets/rounding — hard to make exact, needs an approximation policy), **offset** (2D), and
convenience **up/down/left/right/fwd/back** sugar over `Translate`. None gate the two showcases; logged
here for the roadmap. `hull` is unused in his repo — skip.

## Open questions (for async operator review — non-blocking; I proceed on my best call)
1. `Rotate` as a mesh-leaf f64 transform (matching `Revolve`), `Mirror` exact in-model — OK? (My plan:
   yes; a 90°-exact `Rotate` optimization is a noted follow-up.)
2. Assembly metadata: named-part + BOM record, or "an assembly is just a `Solid`"? (**Resolved by the
   survey: Solid-only is sufficient; no BOM type. Deferred.**)
3. Snowflake magnet feature (the reference adds/subtracts a magnet mount) — include, or keep the
   showcase pure-geometry? (My plan: pure geometry for the showcase; magnet as an optional config knob.)
4. Deferred capabilities (cone, fuller 2D primitives, Minkowski, offset) — roadmap priority? (My plan:
   none gate the showcases; revisit after snowflake + assembly ship.)

## Progress log
- 2026-07-18: doc created. PRNG solved + built (`prng.cdz`, MINSTD LCG, MR sent) — Part 1's capability
  #2 closed. Studied `printing/snowflake` — algorithm understood. Identified capability gap #1
  (`Rotate`/`Mirror`) as the shared critical-path enabler.
- 2026-07-18: assembly survey (Explore over `printing`'s real parts + `rsolid` API) DONE. Confirmed
  "an assembly is just a `Solid`" (no formal mates/part-tree in his repo — it's function composition +
  origin-relative transform chains + mirror symmetry). Sharpened the `Rotate`/`Mirror` design:
  axis-mirror + 90°-rotate are exactly Rational (do Mirror exact in-model), arbitrary angles are
  mesh-leaf f64 (`Revolve` precedent). Logged deferred caps (cone/2D-primitives/Minkowski/offset).
  Assembly-metadata record dropped (no real need). Next: build `Rotate`/`Mirror`, then snowflake.
- 2026-07-18: `Rotate`/`Mirror` BUILT end-to-end — model (`exact.cdz`/`units-model.cdz`/`helpers.cdz`,
  6 @tests) + both mesh drivers (native `cdz-cad` + browser `index.ts`, 7 tests). cad 96/96; cdz-cad
  lib 52/0; guide tsc + 423/423. The shared enabler is done.
- 2026-07-18: OPERATOR refined Part 1 — the PRNG should be an EFFECT (no manual state threading):
  a `Prng` effect performed freely, the LCG state carried in the handler (state-in-a-handler, seeded
  from the `@param` seed). Built it — a SINGLE perform reduces + is deterministic. BLOCKED, though, on a
  real v-effects limitation: **two SEQUENTIAL performs in one handled body** (`let x = roll … in
  let y = roll … in …`) errors "handler not yet reducible by the tail-resumptive fold (non-tail resume
  arrives in a later increment)." A procedural model does many sequential draws, so this is the norm,
  not an edge case. Routed to v-effects with a minimal repro
  (`issues/mlrepro-two-sequential-performs-in-handled-body-non-tail-resume.cdz`). DECISION: build the
  snowflake NOW on the pure state-threaded `prng.cdz` (landed, works for many draws — same deterministic
  unique-per-seed result the operator wants), and SWAP to the effect surface once v-effects lands
  non-tail resume. The showcase ships either way; the effect is an internal ergonomics upgrade.
