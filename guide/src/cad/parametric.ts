/// Parametric CAD showcase — the /cad "super cool" demo: models whose dimensions are `@param` sliders, so
/// "change a number, the model updates", exact (a fractional slider value like 3.5 = 7/2 is carried as an
/// exact Rational a float couldn't hold). This is the DATA half (v-cad owns the content — the model source
/// + the param metadata); /cad (v-guide-infra) reads it to render the sliders + drive the @param
/// host-response → recompute + re-mesh loop (reusing the notebook widget infra). It mirrors how the
/// example-switcher consumes `cad/examples.ts` — a typed, machine-readable manifest, so /cad NEVER parses
/// Cadenza annotations in JS and there's a single source of truth (the .cdz model + this manifest agree;
/// check:visual / check-cad-preload catch any drift).
///
/// UNLIKE the plain examples (which import only `exact`), a parametric model uses the ergonomic HELPERS too,
/// so its `source` carries BOTH imports (`exact` + `helpers`) explicitly — /cad preloads both modules via
/// `compile_with_preloaded`. Each `@param name : Rational` desugars to two scalar host accessors
/// `Param.<name>-num` / `Param.<name>-den` (Int64); /cad supplies each from the slider's value as a num/den
/// pair (an integer slider → den 1; a fractional slider → the exact num/den) and the guest recombines
/// `Rational.of(num, den)`. The `params` array below gives each slider's bounds + default as num/den so the
/// UI needs no Cadenza parsing.

import type { Surface } from "../compiler/client.ts";

/// One `@param` slider's metadata — the machine-readable twin of the `@param(widget: slider, range: […],
/// default: …) name : Rational` annotation in the model source. Bounds/default are given as exact num/den
/// pairs (so a fractional bound is representable); `fractional` hints the UI whether to offer sub-integer
/// steps. `min`/`max`/`default` are `[num, den]` Rationals.
export interface ParamSlider {
  /// The param name — matches the model's `@param … <name>` and the host-response key stem
  /// (`Param.<name>-num` / `Param.<name>-den`).
  name: string;
  /// A human label for the slider.
  label: string;
  /// The slider's minimum / maximum / default value, each an exact `[num, den]` Rational.
  min: [number, number];
  max: [number, number];
  default: [number, number];
  /// Whether the slider should offer fractional (sub-integer) steps (a value like 7/2). All-integer bounds
  /// with `fractional: false` → an integer slider; the model still carries it as a Rational (den 1).
  fractional: boolean;
}

/// A parametric showcase model — the model source (both surfaces, carrying its own exact+helpers imports)
/// plus the slider metadata for its `@param`s. Mirrors `ExampleModel` + a `params` array.
export interface ParametricModel {
  slug: string;
  title: string;
  description: string;
  /// The model source per surface — imports exact + helpers, the `@param` declarations, and `main` returning
  /// `lower(model)`. /cad preloads exact + helpers and drives the `@param`s via host-responses.
  source: Record<Surface, string>;
  /// The `@param` sliders, in declaration order — /cad renders one control per entry + keys each to
  /// `Param.<name>-num` / `Param.<name>-den` host-responses.
  params: ParamSlider[];
}

/// A parametric mounting PLATE — a `width × depth × thickness` block with a central bolt hole of radius
/// `bore` drilled through it. Every dimension is a slider; drag them and the plate resizes + the hole
/// re-bores, all exact. Built with the ergonomic helpers (`box` + `hole-through`). This is the flagship
/// "change a number, the model updates" demo; a fractional thickness (e.g. 3.5 = 7/2) shows the
/// exact-Rational payoff a float slider couldn't represent.
export const MOUNTING_PLATE: ParametricModel = {
  slug: "parametric-plate",
  title: "Parametric mounting plate",
  description: "A width×depth×thickness plate with a central bolt hole — every dimension a live slider, exact.",
  source: {
    ml: `import { Solid, v3, lower } from "exact"
import { box, hole-through } from "helpers"
@!param(widget: slider, range: [20, 200], default: 50) width : Rational
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
    sexpr: `(import "exact" (Solid v3 lower))
(import "helpers" (box hole-through))
(pragma param (param (: widget slider) (: range #list(20 200)) (: default 50)) (: width Rational))
(pragma param (param (: widget slider) (: range #list(20 150)) (: default 30)) (: depth Rational))
(pragma param (param (: widget slider) (: range #list(2 20)) (: default 5)) (: thickness Rational))
(pragma param (param (: widget slider) (: range #list(1 15)) (: default 3)) (: bore Rational))
(def (plate (: w Rational) (: d Rational) (: t Rational) (: r Rational)) (hole-through (box w d t) r t))
(def (main)
  (host (Param)
    (let ((w (Param.width)) (d (Param.depth)) (t (Param.thickness)) (r (Param.bore)))
      (lower (plate w d t r)))))`,
  },
  params: [
    { name: "width", label: "Width", min: [20, 1], max: [200, 1], default: [50, 1], fractional: false },
    { name: "depth", label: "Depth", min: [20, 1], max: [150, 1], default: [30, 1], fractional: false },
    { name: "thickness", label: "Thickness", min: [2, 1], max: [20, 1], default: [5, 1], fractional: true },
    { name: "bore", label: "Bore radius", min: [1, 1], max: [15, 1], default: [3, 1], fractional: false },
  ],
};

/// The parametric showcase models /cad offers (its own affordance, distinct from the static example-picker).
export const PARAMETRIC_MODELS: ParametricModel[] = [MOUNTING_PLATE];

/// The default parametric model /cad opens the parametric view with.
export const DEFAULT_PARAMETRIC = PARAMETRIC_MODELS[0];
