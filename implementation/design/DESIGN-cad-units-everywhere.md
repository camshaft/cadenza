# DESIGN — CAD units-everywhere: a single Rational/Qty geometry model (P1)

*2026-07-16. Operator directive (v-cad reopening, req #2, verbatim): "it's not using the units
everywhere. I would think the Vec3 would be over the Rational+Meter types instead of just plain
rationals. We really want strong typing about units!" — refined live: "I don't want to have 2 different
models, one for floats and one for rationals. It should just all be RATIONALS everywhere."*

> **STATUS: DESIGN — routed to concierge→operator for a ruling BEFORE any code rework (per the P1
> plan: "route the units type-model design to operator BEFORE committing").** No code changed. The
> infra probes below are RUN + green on trunk `cb5dcca53`.

## The one-sentence shape

Geometry coordinates become **length-`Qty` over `Rational`** (a coordinate IS a length, not a bare
number); scale factors stay **bare `Rational`** (dimensionless); there is ONE model (no float twin);
`f64` appears ONLY as a lossy conversion at the manifold-3d FFI edge.

## Why now / what changes

Today `Vec3r` is `V3r(Rational, Rational, Rational)` — bare rationals, with the model unit fixed at
"millimetre" only by CONVENTION (`units.cdz` converts real-world lengths to bare-mm rationals at the
edge). The operator wants the unit carried IN the type so that mixing a length with a bare number — or a
length with a non-length dimension — is a **compile-time type error**, not a silent convention. This is
the Rational+Qty motivator made structural.

## Infra probes (RUN on trunk `cb5dcca53` — the design rests on these, not assumptions)

The units-of-measure design doc lists **Rational-magnitude `Qty` as the one case the operator "punted"
on**. That is STALE for our needs — the operations P1 requires already work over `Qty`-of-`Rational`:

| Probe | Result | Consequence for the model |
|---|---|---|
| `Qty(mm,Rational) + Qty(mm,Rational)` | ✅ PASS (`= 80/1`) | length addition (translate, bbox) works exactly |
| `Qty(mm,Rational) < Qty(mm,Rational)` | ✅ PASS | the bbox min/max fold works (total order preserved) |
| `Qty(mm,Rational) + bare Rational` | ✅ **REJECTED CDZ0301** | **the strong-typing we WANT** — length + number is a type error |
| `Qty(mm,Rational) * Qty(mm,Rational)` | ✅ accepted | ⚠ so a scale factor MUST be bare Rational, else length×length = area |

So P1 is **NOT infra-blocked**. (If a deeper op declines mid-rework — e.g. a specific dimensional
combine over Rational — that's a REAL finding to file against the numeric/quantity track, per the
port-compiler-to-cadenza-ml rule: report, don't work around.)

## The type model

```
type Len  = (Qty Rational Length)      // a coordinate/size: an exact length. (millimetre at the edge)
type Vec3q = V3q(Len, Len, Len)        // the units-carrying position/size vector
```

`Solidr` arms, retyped:
- `Cuber(Vec3q)` — size is three lengths.
- `Spherer(Len)` / `Cylinderr(Len, Len)` — radius/height are lengths.
- `Translater(Vec3q, Solidr)` — offset is a length vector.
- **`Scaler(Vec3r, Solidr)` — factors STAY bare `Rational` (dimensionless).** A scale factor is a pure
  ratio; scaling a length by a length would (by PROBE B) yield area. This is the one arm that is
  deliberately NOT a `Qty` — and the type makes the distinction explicit and enforced.
- `Emptyr` / the three booleans — unchanged (structural).

`Aabbr` (bounding box) carries `Vec3q` corners; `aabbr-size` returns lengths. The min/max fold is
unchanged (PROBE: `<` over length-Qty works).

## The manifold edge (the ONLY f64)

The mesh drivers (native `cdz-cad/mesh.rs`, browser `guide/src/cad/index.ts`) lower each length to a
bare `f64` millimetre magnitude EXACTLY at the `Manifold::cube/sphere/...` call — `Qty.value ∘ Unit.in
mm` then Rational→f64. This is an unavoidable external-lib boundary (manifold is unitless f64), a lossy
EXPORT, NOT a second internal model. Everything above the mesh leaf is `Qty`/`Rational`.

## Numeric mode (app-level, confirmed — NOT a language default)

CAD modules declare `(pragma default-fraction Rational)` (module-scoped; mirrors the calculator's
`assemble_repl_program_exact`). This grounds BOTH bare integer literals (`infer.rs:187`) AND bare
decimals (`infer.rs:256`) to `Rational`, so a CAD author writes natural `n/d` / `0.5` and gets exact
rationals — eliminating today's "bare `n/d` = Int64 division" gotcha. Not a language-wide flip; each app
opts in.

## @param-annotatable (the P4 convergence — design FOR it now)

Per the concierge's operator note: parametric CAD becomes ANNOTATION-DRIVEN — a model dimension marked
`@param(widget=slider, range=0..100mm, type=Length)` and a sidecar (v-effects design + v-metaprogramming
codegen) generates a typed effect (`Param.width : Len`) + a widget manifest. I am the CONSUMER. The
design consequence for P1: **a parametric dimension IS a `Len`-typed input**, so making coordinates
`Len` (not bare Rational) is exactly what lets a dimension be `@param`-annotatable with a typed
accessor. P1's type model is the foundation the annotation layer builds on — no conflict, direct
enabler.

## Staged rollout (each stage gates green independently; NO big-bang)

1. **S1 — numeric pragma.** Add `(pragma default-fraction Rational)` to the CAD modules; authors write
   `n/d` naturally. Small, isolated, gate-green. *(Candidate for the FIRST MR once this design is
   ruled.)*
2. **S2 — `Vec3q`/`Len` + units-carrying constructors** alongside the existing bare-Rational ones (add,
   don't replace — keep the suite green throughout).
3. **S3 — migrate examples + the bbox fold** onto `Vec3q`; `Scaler` stays bare-Rational.
4. **S4 — mesh drivers** lower `Len`→f64 at the manifold leaf (both native + browser).
5. **S5 — retire** the bare-Rational `Vec3r` geometry ctors once nothing depends on them (drivers +
   examples all on `Vec3q`).

## Open questions for the operator

1. **`Len` = millimetre-referenced, or dimension-only (`Length`, unit chosen at the edge)?** Recommend
   dimension-only `Length` internally (the exact model shouldn't privilege mm); the mesh edge picks mm.
2. **Should the model expose `Qty` directly to authors, or a thin `Len` alias?** Recommend a `Len`
   alias for ergonomics + a stable name the `@param` layer can target.
3. **`Scaler` factors bare-Rational — confirm.** (The alternative, a dimensionless-`Qty`, buys nothing
   and complicates the type.)

No code until this is ruled. On a green-light I ship S1 (the pragma) first.
