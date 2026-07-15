# DESIGN — Quantities are reference-normalized; named units are sugar; `as/in` UNWRAPS

*2026-07-15. Operator idea (verbatim): "I'm actually wondering if the `as inches` should UNWRAP
the value out of being a quantity. Like internally converting it to the conversion factor but still
keeping it as meters is the wrong approach and gets you bugs."*

This doc pins the **quantity/conversion model** the operator chose in the design session, and lays
out the increments to move the compiler onto it. It supersedes the storage/display + conversion
semantics currently described in `DESIGN-units-of-measure-rcdzc.md` §7 (families/`Unit.in`) — the
dimensional CORE (Layer 1) is unchanged; what changes is (a) how a Qty is STORED/displayed and (b)
what `Unit.in` / the `as`/`in` surface RETURNS. Read §0 first.

> **STATUS (2026-07-15) — operator decisions locked in this session:**
> 1. **`as/in <unit>` UNWRAPS to a bare dimensionless number** — it converts the magnitude into the
>    target unit AND strips the quantity wrapper. `5 kilometer as inches` → `196850.393…` (a plain
>    number), not a `(Qty T inch)`. This is the operator's "unwrap the value out of being a
>    quantity." It is a deliberate EXIT from the units world.
> 2. **A stored Qty is kept EXACT — normalization is LAZY, never an eager truncation at construction.**
>    *(REVISED 2026-07-15 by operator decision — supersedes the original "eagerly normalize at
>    construction" wording, which truncated Int magnitudes at a fractional reference.)* A quantity
>    retains enough to reproduce its exact value — its source unit's exact scale on the type (the
>    `ty::Unit` already carries `scale_num/scale_den`) — and the scale is applied LAZILY at the point it
>    is observed (display, or a combine that forces a common unit), NOT eagerly at `Qty.of`. So
>    `(Qty.of 2 (Unit.of "foot"))` stays exactly 2 foot and `2 foot in inch` = **24** (the direct
>    source→target ratio, foot/inch = ×12, with NO intermediate truncation through a fractional meter),
>    and `(Qty.of 5.0 (kilo meter))` reads 5000.0 at meter (Float) / stays exact over Rational. The Int
>    magnitude MUST NOT be truncated to the reference at construction: `2 foot` is never silently `0 m`.
>    Float rounds and Rational is exact by their own numeric contracts; Int stays exact by keeping the
>    magnitude in a unit that divides (its source unit), converting only where the ratio is whole or the
>    target is chosen by the program (`Unit.in`).
> 3. **A mixed-unit combine's result carries the dimension's reference unit** (order-independent),
>    reached by each operand's exact scale — unchanged from today (this lazy combine path already works;
>    it is the *construction* path that must NOT pre-normalize).
> 4. **The CALC RELABEL BUG is a DISPLAY bug, not a construction bug** — `5 kilometer` printing
>    `5 meter` (relabel to base without ×1000) is the render dropping the scale, and the fix is to make
>    the single-quantity render number and unit AGREE (either scaled-to-reference `5000 meter` where the
>    numeric type represents it exactly, or the source unit `5 kilometer`), reusing the same
>    scale-aware routine the combine path uses — WITHOUT changing construction.
> 5. **Land this design doc; hand a vertical to the PM.** Implementation (Q1→Q5) is queued.

---

## §0 — The problem, precisely (why the operator's intuition is right)

Today (`ty.rs` `Unit`) a `Qty` is **(erased magnitude, `Unit`)** where a `Unit` is
`{ exponent-map, scale_num/scale_den }` — an exponent map over base dimensions PLUS a machine-int
ratio to the dimension's reference unit. Crucially:

- `5 kilometer` is stored as **magnitude `5`** + unit `{meter:1}` **scale `1000/1`**. The magnitude
  is the number the user WROTE (`5`), in the unit they wrote (`kilometer`) — but the unit's *name*
  ("kilometer") is discarded; only the base-dimension name (`meter`) and the scale `1000` survive.
- The pair is therefore **not self-consistent**: the stored magnitude (`5`) is in kilometers, but
  the only recoverable label is `meter`. Reading it correctly REQUIRES multiplying by the
  side-carried scale. Every consumer must remember to.
- **The display path forgot** (filed bug `mlrepro-calc-bare-quantity-relabels-to-base-without-
  scaling.md`): `5 kilometer` prints `5 meter` — it takes the label (`meter`) but never applies the
  `×1000`. `5 mile` → `5 meter`, `2 km + 3 km` → `5 meter`. The MIXED path is correct
  (`1 km + 500 m` → `1500 m`) precisely because it DOES run the scale multiply — that asymmetry is
  the tell.

The operator's diagnosis: **"keeping it as meters with a factor on the side is the wrong approach and
gets you bugs."** Exactly — a value that is only correct if you remember to apply a stored factor is
a footgun; some code path will always forget. The fix is to make the stored (magnitude, unit) pair
**always self-consistent**, so there is no factor to forget.

## §1 — The model (the two locked decisions, and why they compose)

Two decisions, and together they collapse the representation to something with no hidden state:

### 1a. A stored Qty is kept EXACT; normalization is LAZY, not eager-at-construction

> **⚠ REVISED 2026-07-15 (operator decision).** The original text below proposed EAGER normalization
> at construction (`Qty.of x u` → `(x × u.scale, reference)`). That was implemented as quantity-Q2 and
> found to **truncate Int magnitudes at a fractional reference**: `(Qty.of 2 (Unit.of "foot"))` →
> `2 × 381/1250 = 0` over Int64, so `2 foot in inch` gave `0` instead of `24`. The operator's call:
> **do NOT eagerly normalize; keep the value exact and normalize LAZILY** (at display / at a combine
> that forces a common unit). The `ty::Unit` already carries the exact scale (`scale_num/scale_den`),
> so a quantity retains what it needs to reproduce its exact value; `Unit.in` converts by the direct
> source→target ratio (no intermediate reference truncation), so `2 foot in inch` = 24 exactly. The
> calc relabel bug is a **display** bug (the render drops the scale) — fix the render to make number and
> unit AGREE, not the construction. The eager-at-construction paragraph below is SUPERSEDED; it is kept
> for context (it explains why lazy is required). Int stays exact by keeping its magnitude in a unit
> that divides; Float rounds and Rational is exact by their own numeric contracts.

`Qty.of x u` **eagerly normalizes**: it converts `x` into `u`'s reference unit at construction and
stores `(x × u.scale, u.at_reference())`. So:

- `5 kilometer` = `Qty.of 5 (kilo meter)` → stored `(5000, meter)`.
- `5 mile` → stored `(8046.72…, meter)` (Float) / `40233/5 @ meter` (Rational, exact).
- `1 KiB` → stored `(1024, byte)`.

The stored `Unit` is ALWAYS at scale `1/1` (the reference). **The `scale_num/scale_den` fields of a
STORED unit are always `1/1`** — the scale is consumed at construction, never carried. (Scale still
exists on the *unit VALUES* `Unit.of`/`Unit.prefix` produce — those are compile-time construction
inputs — but a `Ty::Qty`'s unit is always the reference.) The (magnitude, unit) pair is self-
consistent by construction; there is no factor for any consumer to forget.

Named non-reference units (`kilometer`, `foot`, `mbps`, `KiB`, prefixed units) become **pure
construction sugar**: they name a `(scale, reference)` pair, are applied ONCE at `Qty.of`, and never
appear in a stored value or a rendered result. Display always shows the reference unit
(`meter`/`second`/`byte`/…). This is the operator's chosen answer to "keep the written unit vs.
normalize to reference": **normalize to reference for storage AND display.**

### 1b. `as`/`in <unit>` UNWRAPS to a bare dimensionless number

The `as`/`in` surface (and its arena form `Unit.in`) **converts the magnitude into the named target
unit and STRIPS the quantity** — the result is a plain number, not a Qty:

```
5 kilometer as inches   =>  196850.393…        -- a bare number (Float64 / Rational), NOT a Qty
5 km as meters          =>  5000               -- bare number
5 km as inches + 1      =>  196851.393…         -- fine: both bare numbers
5 km as inches + 1 second  => NO dimension error -- the unit was intentionally dropped
```

`as/in <unit>` is the deliberate EXIT from the units world: you asked "how many inches is this?",
you get the number of inches. This is the operator's literal "UNWRAP the value out of being a
quantity." Contrast the two arithmetic worlds:

- **Inside units**: `+ - * / < = compare` are dimension-checked; a length + a time is CDZ0501.
- **After `as/in`**: you hold a bare number; ordinary numeric rules apply; no dimension checking.

So `as/in` is BOTH a representation change (into the named unit's scale) AND the escape hatch. There
is no separate "re-express but stay a Qty" operation in this model — if you want to keep working in
units, you never leave the reference; if you want a specific unit's number, you `as/in` out.

> **Naming:** `in` is the landed keyword (`Unit.in target q`). `as` is the operator's phrasing and a
> natural ML surface synonym. Q4 pins whether we ship `as` as a surface keyword aliasing `in`, or
> keep only `in`. Default: ship `as` as the ML surface spelling (reads naturally: `5 km as inches`),
> desugaring to the same `Unit.in` arena form; `in` remains accepted.

### 1c. A mixed-unit combine's result is the reference unit (unchanged)

`1 km + 500 m` → `1500 meter`, order-independent. This is already how `infer.rs` picks the result
unit (`at_reference()` when scales differ). Under 1a it is now the ONLY case that matters, because
after construction EVERY operand is already at the reference — so `+`/`-` on two same-dimension
quantities are ALWAYS same-unit (both reference), the `ua == ub` branch, and the mixed-scale branch
becomes dead for STORED values (it can still fire on a freshly-constructed literal before folding,
but the answer is identical). This is a simplification, not a new rule.

## §2 — What this CHANGES vs. what's landed (honesty: this revises passing behavior)

This is NOT a pure addition — it revises landed, currently-passing units semantics. The vertical
MUST update the spec + corpus in lockstep (the gate is diff-the-fail-set; these are intentional
output CHANGES, each a spec edit, never a silent todo→fail).

| Behavior | Today (landed) | Under this model |
|---|---|---|
| `5 kilometer` stored/displayed | `5 meter` (BUG: relabel, no scale) | `5000 meter` |
| `5 foot` | `5 meter` (BUG) | `1.524 meter` (Float) / `762/500 @ meter` (Rational) |
| `2 km + 3 km` | `5 meter` (BUG) | `5000 meter` |
| Bare `Qty.of 5.0 (kilo meter)` stored unit | `{meter:1}` scale `1000/1` | `{meter:1}` scale `1/1`, magnitude `5000` |
| `Unit.in (of meter) (Qty.of 1 (of inch))` | `(Qty Rational meter)` = `127/5000 @ meter` | **bare** `127/5000` (unwrapped) |
| `Unit.in (of meter) (Qty.of 3 (kilo meter))` | `(Qty Rational meter)` = `3000/1 @ meter` | **bare** `3000` |
| `5 km as inches + 1 second` | (n/a — no `as`) | bare-number add, no dim error |

The **`Unit.in` return-type change (Qty → bare number) is the big spec revision.** Corpus cases
`18-units-of-measure.sexp:546,589,597,604,652,662` (all the `Unit.in …` outputs) currently expect
`(: (Qty.of N (Unit.base …)) (Qty T …))`; under this model they expect the bare `(: N T)`. The
`Qty.value (Unit.in …)` cases (632-ish, 652, 662) are UNAFFECTED in value (they already unwrap via
`Qty.value`) — but `Unit.in` alone now needs no `Qty.value` to get a scalar.

The **eager-normalize change** flips the stored magnitude of every non-reference construction. The
bare-construction display cases (`70`, `102`, `158` are already meter/reference, unaffected); the
prefixed/family constructions gain the scale in their stored magnitude.

### Spec edits required (the vertical owns these, spec-first)

- `spec/capabilities/units-of-measure.md`: revise §"A Unit Conversion Is The Arithmetic The Source
  Denotes" and the `Unit.in` prose — conversion UNWRAPS to the target unit's scalar count; a stored
  quantity is always at the reference. Add a normative line: *"A conversion `q as/in u` yields the
  dimensionless number of `u` in `q`; the quantity wrapper is removed."*
- `options/units-of-measure/erased-compile-time-quantity.md`: pin the reference-normalized storage
  invariant (a `Ty::Qty`'s unit is always at scale 1/1).
- `spec/semantics/18-units-of-measure.sexp`: update the `Unit.in` outputs to bare numbers, the
  prefixed-construction display outputs to their scaled reference magnitude. **This is the gate.**

## §3 — Seams / file anchors (where each change lands)

The dimensional CORE (`Ty::Qty`, the exponent-map group, CDZ0501, operator dispatch on `Prim`) is
UNCHANGED. Only construction, `Unit.in` typing/lowering, and rendering move.

- **Eager normalize at construction — `infer.rs` `Prim::QtyOf` arm + `lower.rs` `Prim::QtyOf`.**
  Today `QtyOf` reads the unit and builds `Ty::Qty { inner, unit }` verbatim; the value lowers to
  its magnitude UNCHANGED (`lower.rs:1319`). The change: build `Ty::Qty { inner, unit: u.at_reference() }`
  and lower the magnitude to `magnitude × u.scale` (in the inner T — fold when constant, exactly the
  scale multiply the mixed-unit `+` path already emits at `lower.rs:1194`). One shared helper
  `emit_scale(value, num, den, inner_ty)` used by BOTH the construction path and the existing
  mixed-combine path (one source of truth for "apply a scale in the inner numeric type").
- **`Unit.in` → bare number — `infer.rs:3415` + `lower.rs:1355`.** Today `Unit.in target q` types as
  `Ty::Qty { inner, unit: target }` and lowers to `value × (q.scale/target.scale)`. The change: type
  it as the bare `inner` (drop the `Ty::Qty` wrapper); lowering is UNCHANGED (it already computes the
  scalar magnitude — we just stop re-wrapping it as a Qty at the type level). Because a stored `q` is
  now always at the reference (`q.scale == 1/1`), the emitted conversion is `value × (1 / target.scale)`
  = `value / target.scale` — the count of `target` units. (When `target` is the reference,
  `target.scale == 1/1`, a no-op — `q as meters` on a length is identity, correct.)
- **`as` surface keyword — `cadenza-syntax` parser + printer (Q4).** If we ship `as` (default yes):
  a postfix `<expr> as <unit-name>` reads as `(Unit.in (Unit.of #"<unit-name>") <expr>)`, mirroring
  the `5 feet` quantity-literal surface (`DESIGN-units-of-measure-rcdzc.md` §7.5) — parser-only, arena
  unchanged. `in` is already a keyword; `as` joins the small keyword set the quantity-literal parser
  already excludes as unit names. Printer renders the arena `Unit.in` back to `<expr> as <unit>` when
  the target is a bare `Unit.of #"name"` with a bare-safe name (idempotent round-trip), else the call
  form.
- **Rendering — `const_value_ast` Qty arm + the calc display.** A stored Qty renders `<magnitude>
  <reference-unit>` where magnitude is already scaled (no display-time scale needed — the scale was
  consumed at construction). This is what FIXES the filed calc bug at the root: there is no longer a
  factor for the renderer to forget, because there is no factor. The calc `Qty` display reuses the
  same render (one source of truth). The filed bug's `5 kilometer * 1 → 5` (unit dropped) is a
  separate `Mul`-by-bare-number unit-preservation check the vertical verifies in the same pass.

## §4 — Open decisions (with chosen defaults)

1. **Ship `as` as a surface keyword, or keep only `in`?** *Default: ship `as` as the ML surface
   spelling* (`5 km as inches`), desugaring to `Unit.in`; keep `in` accepted. Reads naturally and
   matches the operator's phrasing. If the parser cost of a postfix keyword is high, fall back to
   `in`-only for the first increment and add `as` in a follow-up. — LOW risk, parser-only.

2. **Does `as/in` to the SAME dimension's non-reference unit round-trip exactly?** Yes for Rational
   (exact ratio), lossy for Float/Int (documented: precision lost only where the inner type is
   inexact — the landed spec line). `5000 meter as kilometer` → `5` exactly (Rational) or `5.0`
   (Float). No new rule; falls out of the scale arithmetic.

3. **Should a bare `Qty.of x (reference-unit)` still normalize (a no-op)?** Yes — uniform path,
   `× 1/1` folds away. Byte-neutral for reference-unit constructions (the display cases at `70/102/158`
   stay identical).

4. **Interaction with `Qty.unit` / `Qty.pow` / annotations.** `Qty.unit q` now always returns a
   reference unit (it reads the stored `Ty::Qty.unit`, which is always at reference) — consistent,
   slightly less expressive (you can't recover "the value was in km"), which is the whole point.
   `Qty.pow` composes reference units (unchanged). Annotation `(: e (Qty T u))` with a non-reference
   `u`: the expression DERIVES a reference-unit Qty, so annotating at a non-reference `u`
   should be accepted iff `u`'s dimension matches (the scale is irrelevant to the type identity under
   this model — OR we require the annotation to name the reference unit). *Default: accept any unit of
   the right DIMENSION in an annotation* (dimension is what's checked; scale is construction sugar).
   The vertical pins this against the annotation corpus case.

## §5 — Increment plan (each a landable slice; gate 0-fail, spec-first)

Ordered so the filed calc bug is fixed early and each step is independently gated.

- **Q1 — Spec + corpus revision (spec-first, no compiler change yet).** Edit
  `units-of-measure.md`, `erased-compile-time-quantity.md`, and `18-units-of-measure.sexp` to the new
  model (§2). The corpus cases flip to their new expected outputs and go `todo` (the compiler doesn't
  yet produce them). This pins the contract before the code moves. Gate: the edited cases are `todo`,
  no `fail`.

- **Q2 — Eager normalize at construction (`QtyOf`).** `infer.rs` builds the reference-unit type;
  `lower.rs` emits the scale multiply (shared `emit_scale` helper with the mixed-combine path). Fold
  when constant. **This alone fixes `5 kilometer → 5000 meter`, `5 foot`, `2 km + 3 km`** — the filed
  calc bug's main symptoms. Gate: the prefixed/family construction + display corpus cases flip
  todo→pass; the reference-unit cases stay byte-identical; add a `cdz-calc` regression for
  `5 kilometer` and a homogeneous-prefix sum.

- **Q3 — `Unit.in` → bare number.** `infer.rs:3415` returns the bare `inner`; `lower.rs:1355`
  unchanged (already scalar). The `Unit.in …` corpus cases flip todo→pass (now bare `(: N T)`).
  Verify `5 km as meters` (identity to reference) and a cross-unit conversion. Gate: the `Unit.in`
  cases pass; no dimension check fires on the unwrapped result.

- **Q4 — `as` surface keyword (parser + printer).** Postfix `<expr> as <unit>` → `Unit.in`;
  idempotent round-trip. Parser-only; arena/corpus unchanged (the s-expr cases keep using `Unit.in`).
  Add a `cdz-calc` case `5 km as inches`. Gate: syntax round-trip tests + calc regression.

- **Q5 — Calc display reuse + `* 1` unit preservation.** Point the calc `Qty` render at the shared
  reference-unit renderer (one source of truth, §3); verify `5 kilometer * 1` keeps its unit (the
  separate `Mul`-by-bare-number check the filed bug flags). Gate: the filed bug's full acceptance
  table (`mlrepro-calc-bare-quantity-relabels-to-base-without-scaling.md` §Acceptance) passes.

**Ordering rationale:** Q1 pins the contract; Q2 fixes the reported bug's core (the marquee win);
Q3 realizes the operator's unwrap semantics; Q4 is the `as` sugar; Q5 closes the calc surface + the
adjacent `* 1` issue. Q2 alone resolves the filed calc-bug PM item — so this design's first
increment ALSO discharges the queued bugfix (coordinate with the PM so the two don't double-land).

## §6 — Relationship to the filed bug

The filed `mlrepro-calc-bare-quantity-relabels-to-base-without-scaling.md` is the SYMPTOM; this
design is the ROOT-CAUSE model change. The bug's "make the single-quantity render reuse the same
normalize-to-base-with-factor routine the mixed path uses" is one valid narrow fix (apply the scale
at display). This design goes further and removes the factor ENTIRELY (apply it at construction),
which is the operator's stated preference and eliminates the whole CLASS of "some path forgot the
factor" bugs — the display path, a future serialization path, a future comparison path all become
correct-by-construction. **Recommendation: fold the bugfix into Q2** (the vertical fixes it as the
first increment of the model change) rather than land a display-only patch that Q2 would then
supersede. The PM should hold the bugfix item pending this vertical, or explicitly decide to ship the
narrow display fix first if the calc needs to be correct before the vertical lands.

## §7 — Risk register

- **This revises landed passing behavior** (the `Unit.in` return type; stored magnitudes). Mitigate
  by spec-first (Q1) and diffing the fail-set: every flip must be an INTENTIONAL output change with a
  matching spec edit, never a `todo→fail` surprise. Land Q1 (corpus to the new contract) before the
  compiler moves so the direction is unambiguous.
- **`Unit.in` losing its Qty result removes the "re-express but stay dimensioned" capability.** By
  design (the operator wants unwrap). If a future need arises for "convert but stay a Qty," it's a
  NEW op (`Unit.as-qty`?), not a reversal — don't quietly keep the old behavior on a second spelling
  (that's the garbage-render / two-spellings trap). Note it in the spec as a considered non-goal.
- **⚠ REALIZED RISK (2026-07-15): eager normalize TRUNCATED Int at a fractional reference.** The
  eager-at-construction model (quantity-Q2) computed `x × u.scale` at `Qty.of` in the inner type. Over
  **Int**, `2 foot × (381/1250)` truncates to `0` immediately, so `2 foot in inch` gave `0` not `24` —
  a real precision loss the direct source→target conversion never had. The breaker's cases
  (`2 foot in inch = 24`, `3000-via-foot = 3000`, landed `@7019646a2`) caught it. **Operator decision:
  fix the model — normalize LAZILY, keep the value exact; do NOT truncate at construction.** So this
  risk is now a DESIGN CONSTRAINT: construction never scales an Int magnitude into a unit it does not
  divide. Float rounding-at-observation is still fine (Float is inexact by contract), Rational is exact;
  only the Int-at-fractional-reference path forced the model change from eager to lazy.
- **The calc relabel bug fix is a DISPLAY change, not a construction change.** `5 kilometer` printing
  `5 meter` is the render substituting the base-dimension name while dropping the scale — so the number
  (`5`) and unit (`meter`) disagree. Fix the single-quantity render to reuse the scale-aware
  normalize-to-reference routine the combine path already uses (one source of truth), so it shows a
  number and unit that AGREE (`5000 meter` where the numeric type represents it, or the source unit
  `5 kilometer`). Construction is untouched, so no Int truncation is introduced.
- **`Qty.unit`** returns the quantity's unit as authored/derived (its `ty::Unit`, scale retained), so a
  program can still reconstruct an equivalent quantity; the lazy model does not force a reference-only
  unit the way the eager model did.
- **Land in this worktree, hand to pr-sync via `merge-request`** (fleet single-writer model) — never
  advance trunk directly.
