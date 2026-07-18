/// Pure, React-free bridge from a compiled model's `@param` manifest (`param_manifest`, the wasm binding)
/// to the `ParamSlider` shape `ParametricControls` already renders. This is what makes /cad SINGLE-MODE:
/// instead of a hardcoded `parametric.ts` slider list, /cad reads the manifest of WHATEVER model is in the
/// editor and surfaces a slider per `@param` the model itself declares (operator's ask — "examples declare
/// their own params, those show up in the UI automatically").
///
/// The manifest fields are STRINGS (`param_manifest` renders them so an exact Rational survives the wasm
/// boundary — the exact value crosses at RUN time via the `Param.<name>-num/-den` host-response pair). We
/// parse the bound/default text to `[num, den]` for the slider, keeping the parse SIMPLE + robust: an
/// integer (`"50"`) or a plain fraction (`"7/2"`); anything else (a Rational default rendered as a source
/// expr like `((. Rational of) 1 4)`, before v-metaprog's num/den fast-follow lands) falls back so a
/// control still renders. The exact fraction is never re-derived from display text at run time — the slider
/// value's own `{num,den}` is what drives the model, so a fallback bound only affects the slider RANGE, not
/// the exact value the reader drags to.
///
/// The step (integer vs fractional half-steps) is derived from the DECLARED TYPE, not a per-param flag: a
/// `Rational`/`Qty` param can hold a fraction, so its slider offers half-steps (7/2 reachable — the exact-
/// fraction payoff v-cad called out); an `Int64` param steps in whole units. This replaces the old hardcoded
/// `fractional` boolean with a rule that tracks the model's own type.

import type { ParamManifestEntry } from "../compiler/client.ts";
import type { ParamSlider } from "./parametric.ts";

/// Parse a manifest bound/default STRING to an exact `[num, den]` Rational, or null when it isn't a plain
/// integer / fraction we can read (a source-expr render, a Qty with a unit, etc.). Kept deliberately narrow
/// — the common `@param` writes bare integer or `n/d` bounds; exact Rational bounds get the num/den fields
/// in v-metaprog's fast-follow, at which point this parse is a fallback, not the path.
export function parseRational(text: string | undefined): [number, number] | null {
  if (text === undefined) return null;
  const t = text.trim();
  // Plain integer: "50", "-3".
  if (/^-?\d+$/.test(t)) return [parseInt(t, 10), 1];
  // Plain fraction: "7/2", "-1/4".
  const frac = /^(-?\d+)\s*\/\s*(\d+)$/.exec(t);
  if (frac) {
    const den = parseInt(frac[2], 10);
    if (den !== 0) return [parseInt(frac[1], 10), den];
  }
  return null;
}

/// True when the declared type is a fractional (Rational-family) one — so its slider offers sub-integer
/// (half) steps. `Int64` (or any integer type) steps in whole units. Matches on the reduced type name the
/// manifest carries (`Rational`, `(Qty Rational meter)`, `Length`, …); an integer type name is the else.
export function isFractionalType(typeName: string): boolean {
  return /\bRational\b|\bQty\b|\bLength\b|\bFloat\b|\bDecimal\b/.test(typeName);
}

/// A readable slider label from a param name: `bore-radius` / `bore_radius` → `Bore radius`.
function labelOf(name: string): string {
  const words = name.replace(/[-_]+/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/// Convert one manifest entry to a `ParamSlider`. Bounds/default come from the manifest strings when
/// parseable; when a bound is absent or unparseable, synthesize a sensible range AROUND the default (or a
/// 0..100 fallback) so a control always renders — a model author who omits `range:` still gets a usable
/// slider rather than none. The default falls back to the range midpoint, then 0.
export function sliderFromManifest(entry: ParamManifestEntry): ParamSlider {
  const fractional = isFractionalType(entry.typeName);
  const def = parseRational(entry.default);
  let min = parseRational(entry.rangeLo);
  let max = parseRational(entry.rangeHi);

  // Synthesize a range when the author omitted `range:`. Center it on the default if we have one (so the
  // starting handle sits mid-track), else a neutral 0..100. Keep dens at 1 (whole-number track endpoints).
  if (min === null || max === null) {
    const d = def ? def[0] / def[1] : 50;
    min = min ?? [Math.floor(Math.min(0, d)), 1];
    max = max ?? [Math.ceil(Math.max(100, d * 2)), 1];
  }
  // Default: the manifest value, else the range midpoint, else 0.
  const fallbackMid = Math.round((min[0] / min[1] + max[0] / max[1]) / 2);
  const dflt: [number, number] = def ?? [fallbackMid, 1];

  return { name: entry.name, label: labelOf(entry.name), min, max, default: dflt, fractional };
}

/// The full slider list for a compiled model's manifest, in declaration order — /cad renders one control
/// per entry. An empty manifest (a model with no `@param`) yields no sliders (single-mode shows just the
/// editor + preview for a non-parametric model).
export function slidersFromManifest(entries: ParamManifestEntry[]): ParamSlider[] {
  return entries.map(sliderFromManifest);
}
