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
  description: "An outer cylinder minus a concentric bore — a pipe, via Difference of two cylinders.",
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
  description: "A cube intersected with a sphere — the sphere rounds off every corner (CSG intersection).",
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
  description: "A wide base with a narrower block stacked on top — Union + Translate compose two slabs.",
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
  description: "A straight base + a cubic-Bézier curved top, extruded — a genuinely-curved part via a 2-D path.",
  source: {
    ml: `def main() =
  let arch = cubic-to(line-to(path-start(), v2(8, 0)), v2(0, 0), v2(8, 10), v2(0, 10)) in
  lower(Solid.ExtrudeLinear(Profile.PathProfile(arch), 2))`,
    sexpr: `(def (main)
  (let ((arch (cubic-to (line-to (path-start) (v2 8 0)) (v2 0 0) (v2 8 10) (v2 0 10))))
    (lower ((. Solid ExtrudeLinear) ((. Profile PathProfile) arch) 2))))`,
  },
};

/// A PARAMETRIC mounting plate — a `width × depth × thickness` block with a central bolt hole of radius
/// `bore`, every dimension a `@param`. In SINGLE-MODE this is just another example: the buffer DECLARES its
/// own `@param`s, and /cad auto-surfaces a slider per param (read live from the compiled model's manifest —
/// no hardcoded slider list). Drag a slider and the model recomputes + re-meshes with EXACT (Rational)
/// dimensions — a fractional thickness (3.5 = 7/2) is the exact-fraction payoff a float slider can't hold.
/// Bare like every example: the `import`s (exact + helpers), the `@!default-fraction Rational` pragma, and
/// the `export` are auto-injected by `injectImport`; the buffer shows ONLY the model (@param decls + main).
/// Uses the ergonomic helpers (`box` + `hole-through`) from the injected `helpers` superset. `main` reads
/// each `@param` via the `Param` host accessor; /cad supplies each from its slider's exact {num,den}.
const PARAMETRIC_PLATE: ExampleModel = {
  slug: "parametric-plate",
  title: "Parametric mounting plate (sliders)",
  description: "A width×depth×thickness plate with a central bolt hole — every dimension a live @param slider, exact.",
  source: {
    ml: `@param(widget: slider, range: [20, 200], default: 50) width : Rational
@param(widget: slider, range: [20, 150], default: 30) depth : Rational
@param(widget: slider, range: [2, 20], default: 5) thickness : Rational
@param(widget: slider, range: [1, 15], default: 3) bore : Rational
def plate(w: Rational, d: Rational, t: Rational, r: Rational) = hole-through(box(w, d, t), r, t)
def main() = host Param in
  (let w = Param.width() in
   let d = Param.depth() in
   let t = Param.thickness() in
   let r = Param.bore() in
     lower(plate(w, d, t, r)))`,
    sexpr: `(: (@ (param (: widget slider) (: range (list 20 200)) (: default 50)) width) Rational)
(: (@ (param (: widget slider) (: range (list 20 150)) (: default 30)) depth) Rational)
(: (@ (param (: widget slider) (: range (list 2 20)) (: default 5)) thickness) Rational)
(: (@ (param (: widget slider) (: range (list 1 15)) (: default 3)) bore) Rational)
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
/// float drift, the reason Rational + Qty exist. In single-mode this is just another example whose `@param`s
/// auto-surface as sliders; the difference is each magnitude is fed through `inch` (from the injected `units`
/// superset) so the slider reads inches. Bare like every example (imports + pragma + export auto-injected).
const UNITS_BRACKET: ExampleModel = {
  slug: "units-bracket",
  title: "Imperial bracket (inch sliders)",
  description: "A plate + bolt hole authored in INCHES, converted exactly to model mm — unit-aware sliders, zero float drift.",
  source: {
    ml: `@param(widget: slider, range: [1, 8], default: 3) bwidth : Rational
@param(widget: slider, range: [1, 6], default: 2) bdepth : Rational
@param(widget: slider, range: [1, 4], default: 1) bthickness : Rational
@param(widget: slider, range: [1, 2], default: 1) bbore : Rational
def bracket(w: Rational, d: Rational, t: Rational, r: Rational) =
  hole-through(box(inch(w), inch(d), inch(t)), inch(r), inch(t))
def main() = host Param in
  (let w = Param.bwidth() in
   let d = Param.bdepth() in
   let t = Param.bthickness() in
   let r = Param.bbore() in
     lower(bracket(w, d, t, r)))`,
    sexpr: `(: (@ (param (: widget slider) (: range (list 1 8)) (: default 3)) bwidth) Rational)
(: (@ (param (: widget slider) (: range (list 1 6)) (: default 2)) bdepth) Rational)
(: (@ (param (: widget slider) (: range (list 1 4)) (: default 1)) bthickness) Rational)
(: (@ (param (: widget slider) (: range (list 1 2)) (: default 1)) bbore) Rational)
(def (bracket (: w Rational) (: d Rational) (: t Rational) (: r Rational))
  (hole-through (box (inch w) (inch d) (inch t)) (inch r) (inch t)))
(def (main)
  (host (Param)
    (let ((w (Param.bwidth)) (d (Param.bdepth)) (t (Param.bthickness)) (r (Param.bbore)))
      (lower (bracket w d t r)))))`,
  },
};

/// The example models the /cad example-switcher offers, in display order. Every one is verified to compile
/// + mesh against the preloaded library. Keep the FIRST entry the canonical simple starter (the /cad route
/// opens with `DEFAULT_EXAMPLE`). The parametric plate is one of these — in single-mode a parametric model
/// is just an example that declares `@param`s, and its sliders auto-surface from the compiled manifest.
export const EXAMPLES: ExampleModel[] = [
  CUBE_WITH_DENT,
  HOLLOW_TUBE,
  ROUNDED_CUBE,
  STEPPED_PEDESTAL,
  ARCH_FIN,
  PARAMETRIC_PLATE,
  UNITS_BRACKET,
];

/// The model the /cad route opens with (the canonical cube-with-dent starter).
export const DEFAULT_EXAMPLE = EXAMPLES[0];
