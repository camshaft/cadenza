# DESIGN — runtime parameters via `@param` annotations + a generated host effect

**Status:** DESIGN SKETCH v2 — reworked around the OPERATOR's annotation-driven-codegen direction (2026-07-16;
supersedes the v1 hand-written-monomorphic-ops sketch). Design-first, non-blocking. Author: v-effects (owns the
host-effect seam). Forks go concierge → operator. **Multi-owner:** v-effects (generated host-effect half),
**v-metaprogramming** (the sidecar scan/annotation-processor), **v-syntax** (the `@param` annotation surface).
Consumers: **v-cad** (parametric, driving), **v-notebook** (widgets), G5 sliders, the runtime-input gap.

## The convergence (unchanged) + the operator's richer shape

Four surfaces need *a typed value the host supplies at run time*: v-cad parametric dimensions, v-notebook
interactive widgets, v-cad G5 sliders, the general runtime-input gap. The operator reshaped HOW: not a
hand-written stringly-typed `get("width")`, but **annotation-driven codegen** —

> "a SIDECAR PROGRAM that scans all functions with `@param` annotations and collects them into a SINGLE EFFECT
> that exports a STRONGLY TYPED FUNCTION PER PARAMETER. The attribute specifies which WIDGET to use and RANGE."

### The model (three parts, three owners)

1. **`@param` ANNOTATION (surface — v-syntax).** A function parameter or value is marked `@param` with
   metadata: the WIDGET that renders it + its RANGE. e.g. `@param(widget: slider, range: 0..100mm) width : Length`.
   The annotation carries: widget kind (slider / number / dropdown / checkbox / radio / …), range (min/max/step,
   or enum options), and rides the value's declared TYPE (`Length`, `Int64`, …).
2. **SIDECAR SCAN (codegen — v-metaprogramming).** A build-time pass scans every `@param`-annotated site across
   the program, reads each one's widget/range/type metadata, and GENERATES: (a) a single effect interface whose
   members are one STRONGLY-TYPED accessor per param (`Param.width : Length`, `Param.height : Length` — typed by
   the annotation, NOT `get(String) -> T`); (b) a WIDGET MANIFEST (name → widget + range + type) the host reads
   to render controls. This is annotation-processing over `Ast` — v-metaprogramming's tooling (they own the
   scan + generation).
3. **GENERATED HOST EFFECT (mechanism — v-effects).** The generated effect's accessors are HOST-delegated
   effect ops: `Param.width` performs a host call the host binds at run time (the browser/CAD/notebook supplies
   the current widget value). This is the `(host (Param) …)` → `Core::HostCall` → envelope → `cdz-run` bind path
   v-effects already owns — but the effect INTERFACE is now GENERATED from the annotations, not hand-written.
   The value/typing shape below (from v-cad) still holds — the generated accessor threads the host magnitude
   with the annotation's type.

**One annotation → a typed accessor + a rendered widget.** `@param(slider, 0..100mm) width : Length` yields
`Param.width : Length` (typed, host-bound) AND a slider manifest entry the browser renders. CAD parametric,
notebook widgets, and G5 sliders all fall out of this one mechanism.

## Value/typing shape (SETTLED with v-cad + v-notebook, carries over from v1)

The generated accessor's return crosses the host boundary as v-cad specified — this part is consumer-ratified:
- **Scalar params** (Int64 / Float64 / Bool / String): the host supplies the scalar directly (the shipped
  scalar host-op boundary). Per-call typed — which is now AUTOMATIC (the accessor's type comes from the
  annotation, so the generated op is monomorphic in the right type).
- **Quantity params** (`Length`, …): the host supplies the magnitude as SCALAR INTEGERS — a `(num: i64, den:
  i64)` pair (single i64 for whole numbers) — staying on the SCALAR boundary (NO `_mem`/shared-memory lift).
  The UNIT is guest-side, fixed by the annotation's declared type (`: Length` → the accessor returns a
  length-Qty); the host has NO unit channel, so a wrong-dimension host value is inexpressible; exactness
  preserved (integer num/den, no f64). A host-chosen unit would be the deferred compound-lift escalation.
- **Default fallback** (v-notebook req): a param with no bound host value falls back to a declared default. My
  lean: the default lives guest-side (the annotation or the generated accessor supplies it) so an unbound host
  value is well-defined without the host knowing defaults.
- **Recompute:** PULL (both consumers ratified) — the host re-drives the cell/model on a widget change, the
  program re-performs the accessor, reads the new value. Reactivity stays the consumer's concern (notebook's
  recompute engine, CAD's re-mesh), NOT baked into the effect.

## FORKS — RESOLVED (concierge/operator, 2026-07-16)

- **FORK A → A1, a SEPARATE sidecar PROGRAM** (the operator said "sidecar program"). It emits a generated
  effect module + widget manifest as inspectable artifacts; v-metaprogramming owns it end-to-end. Settled.
- **FORK B → design-collaboration** (v-syntax surface + v-metaprogramming consumer + v-effects effect). Draft a
  CONCRETE `@param` grammar strawman together (see Appendix), then route THAT to concierge → operator for a
  taste-check (concrete beats abstract). In progress — strawman sent to v-syntax + v-metaprogramming.
- **FORK C → my lean** (Qty = num/den scalar-int pair + guest-side annotation-unit, no `_mem` lift, pull-
  recompute). Proceed once B's grammar firms up. Settled.

## Appendix — `@param` grammar STRAWMAN (fork B, for v-syntax/v-metaprogramming co-refinement)

```
@param(widget: slider,   range: 0..100, step: 1)      width  : Length
@param(widget: number,   range: 0.0..10.0)            scale  : Float64
@param(widget: dropdown, options: ["m", "mm", "in"])  unit   : String
@param(widget: toggle,   default: false)              mirror : Bool
@param(widget: slider,   range: 0..100mm, default: 10mm) height : Length
```

Open grammar questions (routed to v-syntax): widget enum (slider/number/dropdown/toggle/radio) + spelling;
RANGE spelling (reuse the `0..100` range-operator surface; unit-suffixed bound `0..100mm` vs unitless+typed;
`step` key vs `0..100:1`); dropdown `options` as a list literal; `default` as an annotation key (the home for
v-notebook's unbound-param fallback — guest-side default); annotation ATTACH point (before a fn param binder
and/or a top-level def/value). Once concrete, the final strawman goes concierge → operator for taste-check.

### Generated-effect INTERFACE the sidecar emits (v-effects spec — v-metaprogramming's codegen TARGET, agreed)

Per `@param` site, the sidecar generates one nullary effect op typed by the annotation's declared type. The
target is STABLE and type-agnostic — codegen always emits `(op <name> (-> Unit <annotation-type>))`:
`(effect Param (op width (-> Unit Length)) (op scale (-> Unit Float64)) (op mirror (-> Unit Bool)) …)` — plus a
widget MANIFEST (`name → widget + range + options + default + type`). That effect DECLARATION + the manifest are
the WHOLE codegen output — no special node. The guest performs `(Param.width)` (nullary perform, elided unit)
under `(host (Param) …)`; v-effects' `perform_host_target` lowers ANY such `(-> Unit R)` op to a `Core::HostCall`
automatically (reads the result type via `op_result_type`), so the plain op decl is the entire target.

**The type line — who emits what** (agreed with v-metaprogramming): codegen emits ONLY the interface
(`(-> Unit R)`); v-effects' mechanism adds the value marshaling.
- Scalar `R` (Int64/Float64/Bool/String): host supplies `R` directly, HostCall result IS `R` — op decl complete.
- Quantity `R` (Length/…): codegen still emits just `(op width (-> Unit Length))` and STOPS. v-effects handles
  the num/den→Qty reconstruction internally (the host ABI supplies the magnitude as a `(num, den)` i64 pair; the
  Quantity-host-op lowering reconstructs the exact Qty + attaches the unit from the declared `Length` type). So
  codegen stays type-agnostic; the Quantity-host-op ABI is the one genuinely-new v-effects bit (scalar host ops
  ship today), built at implementation time — but the interface target is unaffected.

**Bind-by-name = member name** (confirmed): the host binds `Param.<name>` by the string `<name>`, which is the
op member name AND the manifest key — all the same string, so the generated interface and the host bind agree.

### Fork-A refinement (v-metaprogramming, agreed): HYBRID A2-mechanism / A1-artifact. The scan+generate runs as
an in-compiler pass over the RESOLVED Ast (A2 — a strongly-typed accessor needs the `@param`'s declared type,
known only post-resolve; a pure standalone tool would re-run the front-end anyway), but the generated effect
module + widget manifest stay first-class INSPECTABLE artifacts (the A1 virtue the operator wanted). So: a
distinct generate step v-metaprogramming owns, running post-resolve, emitting inspectable outputs.

## FORKS (v1 — superseded by the resolutions above; kept for the record)

- **FORK A — is the sidecar a SEPARATE program/pass or a compiler stage?** The operator said "sidecar program"
  — a distinct scan-and-generate step. Options: (A1) a standalone tool run before compile (emits a generated
  `.cdz`/effect module the main compile consumes) — matches "sidecar program" literally, clean separation,
  v-metaprogramming owns it end-to-end; (A2) an in-compiler annotation-processing STAGE (the scan+generate runs
  as a compiler pass over `Ast`) — tighter integration, no separate artifact, but couples it into the pipeline.
  **Lean: A1** (a sidecar program) — matches the operator's words, keeps the generated-effect artifact
  inspectable, and lets the widget manifest be a first-class output the host reads. v-metaprogramming to weigh in.
- **FORK B — the `@param` annotation grammar (v-syntax).** Widget kinds (slider/number/dropdown/checkbox/radio/
  …) + range spelling (`range: 0..100mm` — reuse the range-operator surface? enum options for dropdown?) +
  how the type annotation attaches (`@param(...) width : Length`). Coordinate with v-syntax (building the
  annotation front-end now).
- **FORK C — how the generated typed accessor threads the host value with the annotation's type**, esp. units
  (a `Length` accessor reconstructs the Qty from the host's num/den + the annotation's unit). This is my seam;
  I'll spec it — flagged as a fork only because it depends on FORK B's type-annotation shape.

## Ownership split

- **v-effects (me):** the generated host effect's MECHANISM — the accessor ops perform host calls, the host
  binds at run time, the value/typing shape above. I spec how a generated accessor lowers to a `HostCall` and
  how the host binds by name, and how the annotation's type flows into the accessor's result type.
- **v-metaprogramming:** the SIDECAR SCAN — find `@param` sites, read metadata, generate the effect interface +
  accessors + widget manifest. Annotation-processing over `Ast`. Likely owns the sidecar end-to-end; consumes
  my generated-effect interface spec.
- **v-syntax:** the `@param(widget:…, range:…)` annotation surface + parse.
- **v-cad / v-notebook:** consumers — mark `@param`, build their surfaces on the generated effect + manifest.

## Recommendation (to route)

Rework accepted. Design = **`@param` annotations → v-metaprogramming sidecar scan → one generated effect with a
strongly-typed accessor per param + a widget manifest → v-effects host-effect mechanism binds each accessor to
the host at run time**, with the consumer-ratified value shape (scalar direct; Qty as num/den scalar-int +
guest-side annotation-unit; pull-recompute; guest default). Forks A (sidecar program vs compiler stage), B
(annotation grammar), C (accessor↔type threading). I own the generated-effect half; looping in v-metaprogramming
(sidecar scan) + v-syntax (annotation surface). NON-urgent to build; unified before the consumers roll their own.
