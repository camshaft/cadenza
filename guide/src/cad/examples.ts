/// Canonical example CAD models — the starter programs the /cad route offers in its example-switcher
/// (operator UX ask: "there's nothing to switch between examples on the notebook or cad program"). Each is
/// a self-contained model built against the PRELOADED CAD library (`Solid`/`v3r`/`lower` from `exact.cdz`,
/// operator P5 ruling A): the reader's buffer holds ONLY the model — BOTH the `import … from "exact"` clause
/// AND the `@!default-fraction Rational` pragma are auto-injected by CadPage's `injectImport` (not shown), so
/// the example buffers are clean (no import, no pragma boilerplate — the operator-directed implicit-default-
/// fraction UX). The injected pragma makes a bare `n/d` an exact Rational (without it `v3r(4/1,…)` rejects
/// CDZ0203); it's module-scoped and does NOT leak into the imported library. Every model returns
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
      Solid.Cube(v3r(4/1, 4/1, 4/1)),
      Solid.Sphere(5/2)))`,
    sexpr: `(def (main)
  (lower ((. Solid Difference)
           ((. Solid Cube) (v3r (/ 4 1) (/ 4 1) (/ 4 1)))
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
      Solid.Cylinder(6/1, 2/1),
      Solid.Cylinder(6/1, 1/1)))`,
    sexpr: `(def (main)
  (lower ((. Solid Difference)
           ((. Solid Cylinder) (/ 6 1) (/ 2 1))
           ((. Solid Cylinder) (/ 6 1) (/ 1 1)))))`,
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
      Solid.Cube(v3r(3/1, 3/1, 3/1)),
      Solid.Sphere(2/1)))`,
    sexpr: `(def (main)
  (lower ((. Solid Intersection)
           ((. Solid Cube) (v3r (/ 3 1) (/ 3 1) (/ 3 1)))
           ((. Solid Sphere) (/ 2 1)))))`,
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
      Solid.Cube(v3r(4/1, 4/1, 1/1)),
      Solid.Translate(
        v3r(0/1, 0/1, 1/1),
        Solid.Cube(v3r(2/1, 2/1, 1/1)))))`,
    sexpr: `(def (main)
  (lower ((. Solid Union)
           ((. Solid Cube) (v3r (/ 4 1) (/ 4 1) (/ 1 1)))
           ((. Solid Translate)
             (v3r (/ 0 1) (/ 0 1) (/ 1 1))
             ((. Solid Cube) (v3r (/ 2 1) (/ 2 1) (/ 1 1)))))))`,
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
  let arch = cubic-to(line-to(path-start(), v2(8/1, 0/1)), v2(0/1, 0/1), v2(8/1, 10/1), v2(0/1, 10/1)) in
  lower(Solid.ExtrudeLinear(Profile.PathProfile(arch), 2/1))`,
    sexpr: `(def (main)
  (let ((arch (cubic-to (line-to (path-start) (v2 (/ 8 1) (/ 0 1))) (v2 (/ 0 1) (/ 0 1)) (v2 (/ 8 1) (/ 10 1)) (v2 (/ 0 1) (/ 10 1)))))
    (lower ((. Solid ExtrudeLinear) ((. Profile PathProfile) arch) (/ 2 1)))))`,
  },
};

/// The example models the /cad example-switcher offers, in display order. Every one is verified to compile
/// + mesh against the preloaded library. Keep the FIRST entry the canonical simple starter (the /cad route
/// opens with `DEFAULT_EXAMPLE`).
export const EXAMPLES: ExampleModel[] = [
  CUBE_WITH_DENT,
  HOLLOW_TUBE,
  ROUNDED_CUBE,
  STEPPED_PEDESTAL,
  ARCH_FIN,
];

/// The model the /cad route opens with (the canonical cube-with-dent starter).
export const DEFAULT_EXAMPLE = EXAMPLES[0];
