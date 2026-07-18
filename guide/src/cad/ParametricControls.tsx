/// Sliders for a PARAMETRIC /cad model's `@param`s (operator's parametric-CAD showcase). Each `ParamSlider`
/// (from `cad/parametric.ts`, v-cad-authored) is a Rational dimension with `[num,den]` bounds/default; this
/// renders one range slider per param and reports its value back as an EXACT fraction `{num,den}` — so a
/// `fractional` param (e.g. thickness) can hold 7/2 = exactly 3.5, the payoff floats can't do. The parent
/// (CadPage) supplies each param's host-response from these `{num,den}` (its step-2 run-worker wiring), so
/// dragging a slider recomputes + re-meshes the model live.
///
/// Purely presentational + an `onChange(name, {num,den})` callback; no compile/run here (the parent drives).

import type { ParamSlider } from "./parametric.ts";

/// A Rational value as an exact fraction. `den` is chosen from the slider's step so a fractional slider
/// yields an exact sub-integer (step 1/2 → den 2, so 3.5 → {num:7, den:2}); an integer slider is den 1.
export interface Frac {
  num: number;
  den: number;
}

/// The DENOMINATOR a `fractional` slider steps in: halves (den 2), so the classic exact-3.5 (7/2) is
/// reachable and the "floats can't hold this" demo lands. An integer slider is den 1 (whole steps). Kept
/// small + fixed (not a free decimal) so every slider value is an exact, legible fraction.
const FRACTIONAL_DEN = 2;

/// A slider's step + denominator: a fractional param steps in halves (0.5, den 2); an integer param steps
/// in whole units (1, den 1).
function stepOf(p: ParamSlider): { step: number; den: number } {
  return p.fractional ? { step: 1 / FRACTIONAL_DEN, den: FRACTIONAL_DEN } : { step: 1, den: 1 };
}

/// A `[num,den]` bound as a plain JS number for the range input's min/max/value (the slider works in
/// decimals; `fracOf` converts the chosen decimal back to an exact fraction).
function toNumber([num, den]: [number, number]): number {
  return den === 0 ? 0 : num / den;
}

/// A slider's decimal value → an exact fraction at the param's denominator (round to the nearest step so a
/// float like 3.4999999 from the range input snaps to 7/2, not 6999/2000).
export function fracOf(value: number, den: number): Frac {
  return { num: Math.round(value * den), den };
}

/// Render the fraction for display in LOWEST TERMS: `n/d` when it isn't a whole number (exact, e.g. `7/2`),
/// else the integer. Reducing means a `fractional` slider parked on a whole value shows `5`, not `10/2`,
/// while a genuine fraction stays exact (`7/2`) — the reader sees the same exact number that meshes.
function showFrac(f: Frac): string {
  const g = gcd(Math.abs(f.num), Math.abs(f.den)) || 1;
  const num = f.num / g;
  const den = f.den / g;
  return den === 1 ? String(num) : `${num}/${den}`;
}

function gcd(a: number, b: number): number {
  return b === 0 ? a : gcd(b, a % b);
}

interface Props {
  params: ParamSlider[];
  /// Current value per param name, as an exact fraction (the parent's source of truth — used as host-responses).
  values: Record<string, Frac>;
  onChange: (name: string, value: Frac) => void;
}

export function ParametricControls({ params, values, onChange }: Props) {
  return (
    <div className="space-y-3" data-testid="cad-param-controls">
      {params.map((p) => {
        const { step, den } = stepOf(p);
        const cur = values[p.name] ?? fracOf(toNumber(p.default), den);
        return (
          <label key={p.name} className="block" data-testid={`cad-param-${p.name}`}>
            <div className="mb-1 flex items-baseline justify-between text-xs">
              <span className="text-slate-300">{p.label}</span>
              {/* The EXACT value crossing to the model — a fraction like 7/2, not a rounded decimal. */}
              <span className="font-mono text-cadenza-300" data-testid={`cad-param-${p.name}-value`}>
                {showFrac(cur)}
              </span>
            </div>
            {/* Mobile: a 44px-tall hit area below `sm` so the slider is comfortable to drag by thumb. */}
            <input
              type="range"
              className="h-11 w-full sm:h-auto"
              min={toNumber(p.min)}
              max={toNumber(p.max)}
              step={step}
              value={cur.num / cur.den}
              onChange={(e) => onChange(p.name, fracOf(Number(e.target.value), den))}
            />
          </label>
        );
      })}
    </div>
  );
}
