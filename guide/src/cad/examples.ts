/// Canonical example CAD models — the starter programs the /cad route offers in its example-switcher
/// (operator UX ask: "there's nothing to switch between examples on the notebook or cad program"). Each is
/// a self-contained model built against the PRELOADED CAD library (`Solid`/`v3`/`lower` from `exact.cdz`,
/// operator P5 ruling A): the reader's buffer holds ONLY the model — BOTH the `import … from "exact"` clause
/// AND the `@!default-fraction Rational` pragma are auto-injected by CadPage's `injectImport` (not shown), so
/// the example buffers are clean (no import, no pragma boilerplate — the operator-directed implicit-default-
/// fraction UX). The injected pragma makes a bare integer literal an exact Rational (so a whole dimension is
/// just `4`, not `4/1` — no divide-by-one noise; a true fraction like `5/2` is written as-is), and a bare
/// `n/d` an exact Rational; without the pragma `v3(4, …)` would reject CDZ0203. The pragma is module-scoped
/// and does NOT leak into the imported library. Every model returns
/// `lower(<Solid model>)` — `exact.cdz`'s `Solid`
/// is GENERIC and a generic value can't be host-rendered yet, so `lower` maps it to the monomorphic
/// `SolidR` the compiler emits + the mesh driver parses. Both surfaces of each example render to the SAME
/// canonical s-expr → the driver meshes them identically.
///
/// This is CONTENT (v-cad owns it — the authority on CAD showcase models); CadPage (v-guide-infra) wires
/// the picker UI that lists these and swaps the editor buffer to the selected example's `[surface]` source.
/// Mirrors the notebook's `examples.ts` shape. Every model here is verified to compile + mesh against the
/// real compiler (via `cdz run` over the preloaded library) — `examples.test.ts` pins the structural
/// invariants (both surfaces present, non-empty, define `main`, return `lower`).

import type { Surface } from "../compiler/client.ts";

export interface ExampleModel {
  /// A stable kebab-case id (the picker's value + a test key).
  slug: string;
  /// The human label shown in the picker.
  title: string;
  /// One line describing the CSG shape, shown alongside the picker.
  description: string;
  /// The model source per surface — the reader edits in whichever surface is selected. Both compile to the
  /// same `SolidR` value, so a mid-view surface toggle re-seeds from the matching string here.
  source: Record<Surface, string>;
}

/// The classic CSG difference — a 4mm cube with a 2.5-radius spherical dent scooped out of it. This is the
/// canonical starter (a cube minus a sphere) and the simplest illustration of `Difference`.
const CUBE_WITH_DENT: ExampleModel = {
  slug: "cube-with-dent",
  title: "Cube with a spherical dent",
  description: "A 4mm cube with a radius-2.5 sphere subtracted (the classic CSG difference).",
  source: {
    ml: `def main() =
  lower(
    Solid.Difference(
      Solid.Cube(v3(4, 4, 4)),
      Solid.Sphere(5/2)))`,
    sexpr: `(def (main)
  (lower ((. Solid Difference)
           ((. Solid Cube) (v3 4 4 4))
           ((. Solid Sphere) (/ 5 2)))))`,
  },
};

/// A hollow tube — an outer cylinder with a concentric bore drilled through it (a `Difference` of two
/// cylinders sharing an axis). A canonical mechanical part (a pipe / bushing).
const HOLLOW_TUBE: ExampleModel = {
  slug: "hollow-tube",
  title: "Hollow tube",
  description: "An outer cylinder minus a concentric bore, making a pipe, via Difference of two cylinders.",
  source: {
    ml: `def main() =
  lower(
    Solid.Difference(
      Solid.Cylinder(6, 2),
      Solid.Cylinder(6, 1)))`,
    sexpr: `(def (main)
  (lower ((. Solid Difference)
           ((. Solid Cylinder) 6 2)
           ((. Solid Cylinder) 6 1))))`,
  },
};

/// A rounded cube — a cube intersected with a sphere, so the sphere clips every corner into a smooth bulge
/// (an `Intersection`, the boolean AND). Shows how intersection carves a shape down to the shared volume.
const ROUNDED_CUBE: ExampleModel = {
  slug: "rounded-cube",
  title: "Rounded cube",
  description: "A cube intersected with a sphere, so the sphere rounds off every corner (CSG intersection).",
  source: {
    ml: `def main() =
  lower(
    Solid.Intersection(
      Solid.Cube(v3(3, 3, 3)),
      Solid.Sphere(2)))`,
    sexpr: `(def (main)
  (lower ((. Solid Intersection)
           ((. Solid Cube) (v3 3 3 3))
           ((. Solid Sphere) 2))))`,
  },
};

/// A stepped pedestal — a wide base slab with a narrower slab stacked on top, joined with `Union` and
/// positioned with `Translate`. Shows composing multiple primitives with a boolean OR + a transform.
const STEPPED_PEDESTAL: ExampleModel = {
  slug: "stepped-pedestal",
  title: "Stepped pedestal",
  description: "A wide base with a narrower block stacked on top, where Union + Translate compose two slabs.",
  source: {
    ml: `def main() =
  lower(
    Solid.Union(
      Solid.Cube(v3(4, 4, 1)),
      Solid.Translate(
        v3(0, 0, 1),
        Solid.Cube(v3(2, 2, 1)))))`,
    sexpr: `(def (main)
  (lower ((. Solid Union)
           ((. Solid Cube) (v3 4 4 1))
           ((. Solid Translate)
             (v3 0 0 1)
             ((. Solid Cube) (v3 2 2 1))))))`,
  },
};

/// A genuinely-CURVED part (v-cad-authored, P-D): an "arch" profile — a straight base + a cubic-Bézier
/// curved top — EXTRUDED to a fin. A 2-D `PathProfile` (path-start → line-to → cubic-to) extruded via
/// `Solid.ExtrudeLinear`; the browser mesh driver (index.ts) samples the Bézier to a polygon + extrudes it.
/// Shows /cad handles free-form curves, not just boxes/cylinders. Uses the injected import superset's 2-D
/// path builders (Profile/path-start/line-to/cubic-to/v2).
const ARCH_FIN: ExampleModel = {
  slug: "arch-fin",
  title: "Arch (cubic-Bézier spline)",
  description: "A straight base + a cubic-Bézier curved top, extruded into a genuinely-curved part via a 2-D path.",
  source: {
    ml: `def main() =
  let arch = cubic-to(line-to(path-start(), v2(8, 0)), v2(0, 0), v2(8, 10), v2(0, 10)) in
  lower(Solid.ExtrudeLinear(Profile.PathProfile(arch), 2))`,
    sexpr: `(def (main)
  (let ((arch (cubic-to (line-to (path-start) (v2 8 0)) (v2 0 0) (v2 8 10) (v2 0 10))))
    (lower ((. Solid ExtrudeLinear) ((. Profile PathProfile) arch) 2))))`,
  },
};

/// A TESSELLATION-RESOLUTION demo (v-cad's OpenSCAD-`$fn` showcase): two identical radius-3 spheres side by
/// side, where the RIGHT one is wrapped in `Solid.Detail(6, …)` — a per-object resolution OVERRIDE pinning it
/// to a coarse 6-segment facet. The LEFT sphere carries no Detail, so it follows the /cad preview Quality
/// slider (the ambient resolution). Drag the slider: the left sphere smooths out while the right stays a
/// faceted gem — the whole point of `$fn`, that resolution CASCADES from a default but a subtree can locally
/// override it (a high-detail seat on a coarse base, etc.). A pure MESH hint — both spheres are the same exact
/// radius-3 geometry, only tessellated differently. `Detail`'s first arg is a segment COUNT (an `Int64`, so
/// `(6 : Int64)`, not a Rational dimension); everything else is the usual exact `Solid` vocabulary.
const DETAIL_OVERRIDE: ExampleModel = {
  slug: "detail-override",
  title: "Tessellation detail ($fn override)",
  description: "Two identical spheres, where the right one is pinned coarse with Solid.Detail(6, ...) while the left follows the Quality slider, so dragging it shows the per-object resolution override.",
  source: {
    ml: `def main() =
  lower(
    Solid.Union(
      Solid.Translate(v3(-5, 0, 0), Solid.Sphere(3)),
      Solid.Translate(v3(5, 0, 0), Solid.Detail((6 : Int64), Solid.Sphere(3)))))`,
    sexpr: `(def (main)
  (lower ((. Solid Union)
           ((. Solid Translate) (v3 -5 0 0) ((. Solid Sphere) 3))
           ((. Solid Translate) (v3 5 0 0) ((. Solid Detail) (: 6 Int64) ((. Solid Sphere) 3))))))`,
  },
};

/// A PARAMETRIC mounting plate — a `width × depth × thickness` block with a central bolt hole of radius
/// `bore`, every dimension a `@!param`. In SINGLE-MODE this is just another example: the buffer DECLARES its
/// own `@!param`s, and /cad auto-surfaces a slider per param (read live from the compiled model's manifest —
/// no hardcoded slider list). Drag a slider and the model recomputes + re-meshes with EXACT (Rational)
/// dimensions — a fractional thickness (3.5 = 7/2) is the exact-fraction payoff a float slider can't hold.
/// Bare like every example: the `import`s (exact + helpers), the `@!default-fraction Rational` pragma, and
/// the `export` are auto-injected by `injectImport`; the buffer shows ONLY the model (@!param decls + main).
/// Uses the ergonomic helpers (`box` + `hole-through`) from the injected `helpers` superset. `main` reads
/// each `@!param` via the `Param` host accessor; /cad supplies each from its slider's exact {num,den}.
const PARAMETRIC_PLATE: ExampleModel = {
  slug: "parametric-plate",
  title: "Parametric mounting plate (sliders)",
  description: "A width×depth×thickness plate with a central bolt hole, every dimension a live @!param slider, exact.",
  source: {
    ml: `@!param(widget: slider, range: [20, 200], default: 50) width : Rational
@!param(widget: slider, range: [20, 150], default: 30) depth : Rational
@!param(widget: slider, range: [2, 20], default: 5) thickness : Rational
@!param(widget: slider, range: [1, 15], default: 3) bore : Rational
def plate(w: Rational, d: Rational, t: Rational, r: Rational) = hole-through(box(w, d, t), r, t)
def main() = host Param in
  (let w = Param.width() in
   let d = Param.depth() in
   let t = Param.thickness() in
   let r = Param.bore() in
     lower(plate(w, d, t, r)))`,
    sexpr: `(pragma param (param (: widget slider) (: range #list(20 200)) (: default 50)) (: width Rational))
(pragma param (param (: widget slider) (: range #list(20 150)) (: default 30)) (: depth Rational))
(pragma param (param (: widget slider) (: range #list(2 20)) (: default 5)) (: thickness Rational))
(pragma param (param (: widget slider) (: range #list(1 15)) (: default 3)) (: bore Rational))
(def (plate (: w Rational) (: d Rational) (: t Rational) (: r Rational)) (hole-through (box w d t) r t))
(def (main)
  (host (Param)
    (let ((w (Param.width)) (d (Param.depth)) (t (Param.thickness)) (r (Param.bore)))
      (lower (plate w d t r)))))`,
  },
};

/// A UNITS-PARAMETRIC imperial bracket (v-cad's P4 showcase, `showcase-units-parametric.cdz`) — sliders whose
/// values are read in INCHES and converted, exactly over Rational, to the model's millimetres via `inch`. A
/// quarter-inch bore is 127/20 mm exactly; a 3-inch plate is 381/5 mm exactly — mixed-unit authoring with zero
/// float drift, the reason Rational + Qty exist. In single-mode this is just another example whose `@!param`s
/// auto-surface as sliders; the difference is each magnitude is fed through `inch` (from the injected `units`
/// superset) so the slider reads inches. Bare like every example (imports + pragma + export auto-injected).
const UNITS_BRACKET: ExampleModel = {
  slug: "units-bracket",
  title: "Imperial bracket (inch sliders)",
  description: "A plate + bolt hole authored in INCHES, converted exactly to model mm, with unit-aware sliders and zero float drift.",
  source: {
    ml: `@!param(widget: slider, range: [1, 8], default: 3) bwidth : Rational
@!param(widget: slider, range: [1, 6], default: 2) bdepth : Rational
@!param(widget: slider, range: [1, 4], default: 1) bthickness : Rational
@!param(widget: slider, range: [1, 2], default: 1) bbore : Rational
def bracket(w: Rational, d: Rational, t: Rational, r: Rational) =
  hole-through(box(inch(w), inch(d), inch(t)), inch(r), inch(t))
def main() = host Param in
  (let w = Param.bwidth() in
   let d = Param.bdepth() in
   let t = Param.bthickness() in
   let r = Param.bbore() in
     lower(bracket(w, d, t, r)))`,
    sexpr: `(pragma param (param (: widget slider) (: range #list(1 8)) (: default 3)) (: bwidth Rational))
(pragma param (param (: widget slider) (: range #list(1 6)) (: default 2)) (: bdepth Rational))
(pragma param (param (: widget slider) (: range #list(1 4)) (: default 1)) (: bthickness Rational))
(pragma param (param (: widget slider) (: range #list(1 2)) (: default 1)) (: bbore Rational))
(def (bracket (: w Rational) (: d Rational) (: t Rational) (: r Rational))
  (hole-through (box (inch w) (inch d) (inch t)) (inch r) (inch t)))
(def (main)
  (host (Param)
    (let ((w (Param.bwidth)) (d (Param.bdepth)) (t (Param.bthickness)) (r (Param.bbore)))
      (lower (bracket w d t r)))))`,
  },
};

/// An ASSEMBLY-as-code L-bracket (v-cad's assembly showcase, distilled for the picker): a base plate (with a
/// bolt hole) + a vertical arm, mated at 90° via `rotate-x` and combined with `fuse` — the "multiple parts
/// combined in code" half of the operator's assembly ask. Uses the `rotate-x`/`fuse`/`cut`/`move-*`/`box`/
/// `cyl` helpers (from the injected `helpers` superset — the rotation builders were added to CAD_HELPER_NAMES
/// for this). v-cad's mesh driver handles Rotate/Mirror; here it's just another bare example in the picker.
const ASSEMBLY_L_BRACKET: ExampleModel = {
  slug: "assembly-l-bracket",
  title: "L-bracket (assembly-as-code)",
  description: "A base plate + a vertical arm mated at 90 degrees, multiple parts combined in code.",
  source: {
    ml: `def base() = cut(move-z(2, box(40, 30, 4)), move-x(10, move-z(2, cyl(8, 3))))
def arm() = move-z(4, rotate-x(90, move-z(2, box(30, 25, 4))))
def main() = lower(fuse(base(), arm()))`,
    sexpr: `(def (base) (cut (move-z 2 (box 40 30 4)) (move-x 10 (move-z 2 (cyl 8 3)))))
(def (arm) (move-z 4 (rotate-x 90 (move-z 2 (box 30 25 4)))))
(def (main) (lower (fuse (base) (arm))))`,
  },
};

/// A PARAMETRIC ASSEMBLY (v-cad's showcase-assembly-parametric.cdz, distilled): the L-bracket — a base plate
/// + a 90°-rotated standing arm, each with a bolt hole — where EVERY dimension is a `@!param` slider. This is
/// the one example that combines BOTH the assembly-as-code story (rotate-x mated parts, fuse) AND the
/// parametric story (5 live sliders, exact Rational dims) — neither the plain L-bracket nor the mounting
/// plate does both. Single-mode auto-surfaces its 5 @!params (pa-len/wid/thick/rise/bolt); the transforms
/// (rotate-x/move-*/cut/fuse/box/cyl) are all in the injected `helpers` superset. Mesh = v-cad's driver.
const ASSEMBLY_PARAMETRIC_BRACKET: ExampleModel = {
  slug: "assembly-parametric-bracket",
  title: "Parametric L-bracket (assembly + sliders)",
  description: "An L-bracket assembly (base + a 90-degree arm) with every dimension a live @!param slider.",
  source: {
    ml: `@!param(widget: slider, range: [20, 80], default: 40) pa-len : Rational
@!param(widget: slider, range: [15, 50], default: 30) pa-wid : Rational
@!param(widget: slider, range: [2, 10], default: 4) pa-thick : Rational
@!param(widget: slider, range: [10, 50], default: 25) pa-rise : Rational
@!param(widget: slider, range: [1, 8], default: 3) pa-bolt : Rational
def base-plate(len: Rational, wid: Rational, t: Rational, r: Rational) =
  cut(move-z(t / (2 / 1), box(len, wid, t)), move-z(t / (2 / 1), move-x(len / (4 / 1), cyl(t * (2 / 1), r))))
def arm-flat(wid: Rational, rise: Rational, t: Rational, r: Rational) =
  cut(move-z(t / (2 / 1), box(wid, rise, t)), move-z(t / (2 / 1), move-y(rise / (3 / 1), cyl(t * (2 / 1), r))))
def standing-arm(wid: Rational, rise: Rational, t: Rational, r: Rational) =
  move-y((0 / 1 - wid) / (2 / 1), move-z(t, rotate-x(90 / 1, arm-flat(wid, rise, t, r))))
def main() = host Param in
  (let len = Param.pa-len() in let wid = Param.pa-wid() in let t = Param.pa-thick() in let rise = Param.pa-rise() in let r = Param.pa-bolt() in
     lower(fuse(base-plate(len, wid, t, r), standing-arm(wid, rise, t, r))))`,
    sexpr: `(pragma param (param (: widget slider) (: range #list(20 80)) (: default 40)) (: pa-len Rational))
(pragma param (param (: widget slider) (: range #list(15 50)) (: default 30)) (: pa-wid Rational))
(pragma param (param (: widget slider) (: range #list(2 10)) (: default 4)) (: pa-thick Rational))
(pragma param (param (: widget slider) (: range #list(10 50)) (: default 25)) (: pa-rise Rational))
(pragma param (param (: widget slider) (: range #list(1 8)) (: default 3)) (: pa-bolt Rational))
(def (base-plate (: len Rational) (: wid Rational) (: t Rational) (: r Rational))
  (cut (move-z (/ t (/ 2 1)) (box len wid t)) (move-z (/ t (/ 2 1)) (move-x (/ len (/ 4 1)) (cyl (* t (/ 2 1)) r)))))
(def (arm-flat (: wid Rational) (: rise Rational) (: t Rational) (: r Rational))
  (cut (move-z (/ t (/ 2 1)) (box wid rise t)) (move-z (/ t (/ 2 1)) (move-y (/ rise (/ 3 1)) (cyl (* t (/ 2 1)) r)))))
(def (standing-arm (: wid Rational) (: rise Rational) (: t Rational) (: r Rational))
  (move-y (/ (- (/ 0 1) wid) (/ 2 1)) (move-z t (rotate-x (/ 90 1) (arm-flat wid rise t r)))))
(def (main)
  (host (Param)
    (let ((len ((. Param pa-len))) (wid ((. Param pa-wid))) (t ((. Param pa-thick))) (rise ((. Param pa-rise))) (r ((. Param pa-bolt))))
      (lower (fuse (base-plate len wid t r) (standing-arm wid rise t r))))))`,
  },
};

/// The PARAMETRIC SNOWFLAKE (v-cad's flagship showcase — the operator's "seed → unique snowflake"): feed a
/// seed and a seeded PRNG grows a unique, deterministic 6-fold snowflake; sliders drive the seed, arm length,
/// and recursion depth. SELF-CONTAINED (operator directive: "you can create a snowflake from simple
/// primitives! don't hide the whole thing behind a library"): the WHOLE construction is IN the buffer — a
/// MINSTD LCG (`lcg-next`/`roll`/`seed-state`, exact Int64, state threaded explicitly), a branch `segment`
/// built from a `box` bar + a `ball` tip, recursive `branch`/`add-children` with bilateral `mirror-x` and a
/// random child count, and `six-fold` unioning six z-rotated copies. NO opaque `snowflake` lib import — only
/// the `exact`+`helpers` vocab `injectImport` adds (box/ball/fuse/move-x/rotate-z/mirror-x + Solid/lower).
/// Single-mode auto-surfaces the 3 sliders (seed/arm-length/depth). Mesh = v-cad's driver. NOTE: the model
/// folds from `Solid.Empty` (the union identity) — meshing that requires the empty-Solid mesh fix (index.ts
/// `M.union([])`, NOT a zero-size cube that annihilates the boolean); both verified to mesh to ~25k verts.
const PARAMETRIC_SNOWFLAKE: ExampleModel = {
  slug: "parametric-snowflake",
  title: "Parametric snowflake (seed → unique)",
  description: "Feed a seed; a seeded PRNG grows a unique deterministic 6-fold snowflake, built from primitives. Sliders: seed, arm-length, depth.",
  source: {
    ml: `@!param(widget: slider, range: [1, 200], default: 42) seed : Int64
@!param(widget: slider, range: [10, 40], default: 20) arm-length : Rational
@!param(widget: slider, range: [1, 3], default: 2) depth : Int64
def lcg-next(s: Int64) = ((16807 : Int64) * s) % (2147483647 : Int64)
def roll(s: Int64, lo: Int64, hi: Int64) = lo + (lcg-next(s) % (((hi - lo) + (1 : Int64))))
def seed-state(n: Int64) = (n % (2147483646 : Int64)) + (1 : Int64)
def r(n: Int64) = Rational.of(n, (1 : Int64))
def segment(len: Rational) =
  let w = len / (8 / 1) in
  fuse(move-x(len / (2 / 1), box(len, w, w)), move-x(len, ball(w)))
def branch(state: Int64, len: Rational, depth: Int64) =
  if depth == (0 : Int64) then (segment(len), lcg-next(state))
  else
    let n = roll(state, (1 : Int64), (3 : Int64)) in
    add-children(lcg-next(state), len, depth, n, (0 : Int64), segment(len))
def add-children(state: Int64, len: Rational, depth: Int64, n: Int64, i: Int64, acc: Solid(Rational)) =
  if i == n then (acc, state)
  else
    let ang = r(roll(state, (25 : Int64), (75 : Int64))) in
    let s1 = lcg-next(state) in
    let offset = (len * r(roll(s1, (30 : Int64), (70 : Int64)))) / (100 / 1) in
    let s2 = lcg-next(s1) in
    match branch(s2, len * (3 / 5), depth - (1 : Int64)) with
      | (child, s3) =>
        let placed = move-x(offset, rotate-z(ang, child)) in
        add-children(s3, len, depth, n, i + (1 : Int64), fuse(acc, fuse(placed, mirror-x(placed))))
def six-fold(arm: Solid(Rational), i: Int64, acc: Solid(Rational)) =
  if i == (6 : Int64) then acc
  else six-fold(arm, i + (1 : Int64), fuse(acc, rotate-z(r(i * (60 : Int64)), arm)))
def snowflake(seed0: Int64, len: Rational, depth: Int64) =
  match branch(seed-state(seed0), len, depth) with
    | (arm, _) => six-fold(arm, (0 : Int64), Solid.Empty)
def main() = host Param in
  (let s = Param.seed() in
   let len = Param.arm-length() in
   let d = Param.depth() in
     lower(snowflake(s, len, d)))`,
    sexpr: `(pragma param (param (: widget slider) (: range #list(1 200)) (: default 42)) (: seed Int64))
(pragma param (param (: widget slider) (: range #list(10 40)) (: default 20)) (: arm-length Rational))
(pragma param (param (: widget slider) (: range #list(1 3)) (: default 2)) (: depth Int64))
(def (lcg-next (: s Int64)) (% (* (: 16807 Int64) s) (: 2147483647 Int64)))
(def (roll (: s Int64) (: lo Int64) (: hi Int64)) (+ lo (% (lcg-next s) (+ (- hi lo) (: 1 Int64)))))
(def (seed-state (: n Int64)) (+ (% n (: 2147483646 Int64)) (: 1 Int64)))
(def (r (: n Int64)) ((. Rational of) n (: 1 Int64)))
(def (segment (: len Rational))
  (let ((w (/ len (/ 8 1)))) (fuse (move-x (/ len (/ 2 1)) (box len w w)) (move-x len (ball w)))))
(def (branch (: state Int64) (: len Rational) (: depth Int64))
  (if (= depth (: 0 Int64))
    #tuple((segment len) (lcg-next state))
    (let ((n (roll state (: 1 Int64) (: 3 Int64))))
      (add-children (lcg-next state) len depth n (: 0 Int64) (segment len)))))
(def (add-children (: state Int64) (: len Rational) (: depth Int64) (: n Int64) (: i Int64) (: acc (Solid Rational)))
  (if (= i n)
    #tuple(acc state)
    (let ((ang (r (roll state (: 25 Int64) (: 75 Int64)))))
      (let ((s1 (lcg-next state)))
        (let ((offset (/ (* len (r (roll s1 (: 30 Int64) (: 70 Int64)))) (/ 100 1))))
          (let ((s2 (lcg-next s1)))
            (match (branch s2 (* len (/ 3 5)) (- depth (: 1 Int64)))
              (#tuple(child s3)
                (let ((placed (move-x offset (rotate-z ang child))))
                  (add-children s3 len depth n (+ i (: 1 Int64)) (fuse acc (fuse placed (mirror-x placed)))))))))))))
(def (six-fold (: arm (Solid Rational)) (: i Int64) (: acc (Solid Rational)))
  (if (= i (: 6 Int64))
    acc
    (six-fold arm (+ i (: 1 Int64)) (fuse acc (rotate-z (r (* i (: 60 Int64))) arm)))))
(def (snowflake (: seed0 Int64) (: len Rational) (: depth Int64))
  (match (branch (seed-state seed0) len depth)
    (#tuple(arm _) (six-fold arm (: 0 Int64) (. Solid Empty)))))
(def (main)
  (host (Param)
    (let ((s ((. Param seed))) (len ((. Param arm-length))) (d ((. Param depth))))
      (lower (snowflake s len d)))))`,
  },
};

/// A PARAMETRIC POKÉBALL STAND (a printable display cradle, exercising revolve + a spherical seat): a squat
/// truncated cone (frustum) with a spherical DIMPLE carved into its flat top, so a printed ball nestles in a
/// matching seat. There is no cone primitive — the taper is a lathe: a trapezoid `PathProfile`
/// (path-start → line-to·3, from the base rim out at (base/2, 0) up to the top rim (top/2, height) and back
/// down the revolve axis) swept 360° with `Solid.Revolve`. The seat is `cut(frustum, move-z(…, ball(ball/2)))`
/// — a sphere of the ball's own radius, raised so it bites `dimple-depth` into the top face. 🪤 The mesh
/// driver stands a revolve up along +Z (base flat on z=0), so the seat is positioned with `move-z`, not
/// move-y (which would shave a flank). Five live sliders: base Ø, top Ø, height, ball Ø, dimple depth — drag
/// to dial the cradle to a real ball, exact over Rational. Defaults: Ø90 base → Ø66 top, 40 tall, for an Ø80
/// ball, 6mm dimple (a ~Ø42 seat mouth). Uses the injected exact + helpers vocab (Solid/Profile/path-start/
/// line-to/v2/cut/move-z/ball/lower).
const POKEBALL_STAND: ExampleModel = {
  slug: "pokeball-stand",
  title: "Pokéball stand (revolved cone + dimple)",
  description: "A squat truncated cone with a spherical seat dimpled into the top to cradle a printed ball; five live sliders.",
  source: {
    ml: `@!param(widget: slider, range: [60, 120], default: 90) base-dia : Rational
@!param(widget: slider, range: [40, 100], default: 66) top-dia : Rational
@!param(widget: slider, range: [20, 80], default: 40) height : Rational
@!param(widget: slider, range: [50, 110], default: 80) ball-dia : Rational
@!param(widget: slider, range: [2, 20], default: 6) dimple-depth : Rational
def profile(br: Rational, tr: Rational, h: Rational) =
  Profile.PathProfile(line-to(line-to(line-to(path-start(), v2(br, 0)), v2(tr, h)), v2(0, h)))
def stand(bd: Rational, td: Rational, h: Rational, bald: Rational, dd: Rational) =
  let br = bd / 2 in
  let tr = td / 2 in
  let r = bald / 2 in
  let frustum = Solid.Revolve(profile(br, tr, h), 360) in
  cut(frustum, move-z(h + (r - dd), ball(r)))
def main() = host Param in
  (let bd = Param.base-dia() in
   let td = Param.top-dia() in
   let h = Param.height() in
   let bald = Param.ball-dia() in
   let dd = Param.dimple-depth() in
     lower(stand(bd, td, h, bald, dd)))`,
    sexpr: `(pragma param (param (: widget slider) (: range #list(60 120)) (: default 90)) (: base-dia Rational))
(pragma param (param (: widget slider) (: range #list(40 100)) (: default 66)) (: top-dia Rational))
(pragma param (param (: widget slider) (: range #list(20 80)) (: default 40)) (: height Rational))
(pragma param (param (: widget slider) (: range #list(50 110)) (: default 80)) (: ball-dia Rational))
(pragma param (param (: widget slider) (: range #list(2 20)) (: default 6)) (: dimple-depth Rational))
(def (profile (: br Rational) (: tr Rational) (: h Rational))
  ((. Profile PathProfile) (line-to (line-to (line-to (path-start) (v2 br 0)) (v2 tr h)) (v2 0 h))))
(def (stand (: bd Rational) (: td Rational) (: h Rational) (: bald Rational) (: dd Rational))
  (let ((br (/ bd 2)))
    (let ((tr (/ td 2)))
      (let ((r (/ bald 2)))
        (let ((frustum ((. Solid Revolve) (profile br tr h) 360)))
          (cut frustum (move-z (+ h (- r dd)) (ball r))))))))
(def (main)
  (host (Param)
    (let ((bd ((. Param base-dia))))
      (let ((td ((. Param top-dia))))
        (let ((h ((. Param height))))
          (let ((bald ((. Param ball-dia))))
            (let ((dd ((. Param dimple-depth)))) (lower (stand bd td h bald dd)))))))))`,
  },
};

/// The example models the /cad example-switcher offers, in display order. Every one is verified to compile
/// + mesh against the preloaded library. Keep the FIRST entry the canonical simple starter (the /cad route
/// opens with `DEFAULT_EXAMPLE`). The parametric plate is one of these — in single-mode a parametric model
/// is just an example that declares `@!param`s, and its sliders auto-surface from the compiled manifest.
export const EXAMPLES: ExampleModel[] = [
  CUBE_WITH_DENT,
  HOLLOW_TUBE,
  ROUNDED_CUBE,
  STEPPED_PEDESTAL,
  ARCH_FIN,
  DETAIL_OVERRIDE,
  PARAMETRIC_PLATE,
  UNITS_BRACKET,
  ASSEMBLY_L_BRACKET,
  ASSEMBLY_PARAMETRIC_BRACKET,
  PARAMETRIC_SNOWFLAKE,
  POKEBALL_STAND,
];

/// The model the /cad route opens with (the canonical cube-with-dent starter).
export const DEFAULT_EXAMPLE = EXAMPLES[0];
