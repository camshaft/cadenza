# DESIGN — wasm-opt optimality-gap analysis (rcdzc emit quality)

Owner: `v-wasm-opt`. Companion to the nix mechanism owned by `v-nix`
(`mkCorpusOptGap` → the generated aggregate `wasm-opt-gaps.sexp`).

The aggregate is emitted as **s-expressions, not markdown** — it is DATA an agent
parses, ranks, and iterates against, and this codebase is homoiconic sexpr (the
whole corpus + harness already read sexpr), so a `.sexp` gap record is trivially
greppable and manipulable where a prose table is not. The record schema is fixed
in "Gap-record output format" below.

## Goal

For every wasm corpus output that PASSES, measure the gap between what `rcdzc`
emits and what Binaryen's `wasm-opt` would produce. If `wasm-opt` finds nothing
to shrink, our module is OPTIMAL for the metrics we track. Where `wasm-opt` DOES
improve, that delta is a *gap* — a tracked TODO for an emit-side backend
optimization in this vertical's lane.

**A gap is ADVISORY — never a gate-fail.** `wasm-opt` re-encodes and applies
transforms we may deliberately not want; the gap doc is a prioritized backlog,
not a correctness gate. Behavior correctness stays owned by the existing
behavior corpus (`gate --check`); this analysis never blocks a merge.

## Signal: size + `--metrics` delta, NOT byte identity

`wasm-opt` re-encodes the module from its own IR, so its output is byte-different
from ours even when it changed nothing semantically (different LEB padding, local
ordering, block structure). **Byte-diff is therefore a FALSE gap signal.** The
real signal is:

1. **Size delta** — `len(ours) - len(wasm-opt -Oz/-O3 output)`, in bytes. Zero
   (or ≤ a small re-encode epsilon) ⇒ optimal on size.
2. **`--metrics` delta** — run `wasm-opt --metrics` on BOTH our module and the
   optimized module and diff the per-category counts. This tells you *what*
   shrank (which is the actual actionable information), e.g. `funcs 2→1`
   (dead/merged function), `LocalGet 13→10` (redundant local loads removed),
   `vars 3→2` (local coalescing), `Block 2→1` (block merge), `Drop N→M`
   (drop elimination).

Report BOTH: size ranks the gaps; metrics classifies each one.

## Pipeline (what the mechanism must do per case) — GROUNDED, verified 2026-08-27

The single most important structural fact, verified on a real emit:

> **`rcdzc` emits a COMPONENT (binary version `0x0d`), and Binaryen does NOT
> support components** — `wasm-opt` hard-errors on our `emit.wasm`:
> `parse exception: this looks like a wasm component, which Binaryen does not
> support yet` (binaryen#6728).

So the mechanism CANNOT run `wasm-opt` on `emit.wasm` directly. Per-case steps:

1. **Extract the core module(s)** from the component with `wasm-tools`:
   ```
   wasm-tools component unbundle emit.wasm --module-dir <dir> --threshold 0 -o /dev/null
   ```
   This writes each embedded core module as `unbundled-moduleN.wasm`. For an
   ordinary program that is one core module (`unbundled-module0.wasm`); a program
   with a resource escape / dtor stub emits more than one — analyze each, and the
   aggregate size is the sum. (`--threshold 0` extracts even tiny modules.)

2. **Run `wasm-opt` with the emit's feature set.** Our core uses `return_call`
   (tail calls), so a bare `wasm-opt` fails the validator with
   `return_call* requires tail calls [--enable-tail-call]`. Use `--all-features`
   (superset; simplest and stable) so the feature set never silently excludes a
   real gap. If we ever want to attribute a gap to a specific feature, narrow the
   flags, but `--all-features` is the default for the sweep.

3. **Measure.** `wasm-opt --all-features -O3` (and separately `-Oz` for the
   size-focused number) → compare output size to the extracted module size; run
   `--metrics` on both and diff.

4. **Content-address** on `{emit.wasm, binaryen-version}` so a case is only
   re-analyzed when its emit or the tool changes (v-nix owns this caching in the
   flake).

Binaryen version pinning matters: metrics category names/counts can shift across
versions. Pin the binaryen input and record its version in the aggregate doc.

## Gap taxonomy (classify each gap by its `--metrics` signature)

| Gap kind                     | Metrics signature                                  | Emit-side fix lane |
|------------------------------|----------------------------------------------------|--------------------|
| Redundant local traffic      | `LocalGet`↓ `LocalSet`↓ `vars`↓ with size↓         | operand-stack reuse / copy-prop / slot coalescing in the wasm lowering |
| Dead / mergeable function    | `funcs`↓                                           | don't emit trivially-forwarding wrappers; inline single-call helpers   |
| Dead code                    | `total`↓ with a whole construct gone               | drop unreachable arms / never-read binds before emit |
| Block / control merge        | `Block`↓ `If`↓ `Break`↓                            | flatten single-entry blocks; select-ification of tiny if/else |
| Const folding                | `Const`↓ `Binary`↓                                 | fold at emit (coordinate w/ compiler-primitives const-fold — don't dup) |
| Drop elimination             | `Drop`↓                                            | don't push-then-drop; elide the producer |
| br_table / switch            | many `If`→ one `Switch`                            | emit `br_table` for dense integer dispatch |

Rank the whole corpus's gaps by **size delta descending** — the biggest single
byte win is worked first. Group by gap kind so one emit-side fix can close a
whole class across many cases at once.

## First grounded finding (2026-08-27, binaryen 131, `--all-features`)

Probe program (recursive numeric, imports no runtime, non-foldable):
```
(do (def (sum (: n Int64) (: acc Int64)) (if (= n 0) acc (sum (- n 1) (+ acc n))))
    (def (main (: n Int64)) (sum n 0))
    (export main))
```
Extracted core module: **131 B → `-O3` 111 B = −20 B (−15%)**. Metrics delta:
`funcs 2→1`, `vars 3→2`, `LocalGet 13→10`, `LocalSet 5→3`, `Const 7→6`,
`Block 2→1`, `total 45→37`.

Reading: (a) one of our two emitted functions is dead/mergeable after the direct
`main→sum` tail-call (**dead/mergeable function**); (b) we emit extra
`local.set`/`local.get` pairs that Binaryen coalesces away (**redundant local
traffic** — the highest-frequency category and the first class to attack on the
emit side). These are the two candidate gap classes to prioritize once v-nix's
sweep populates the aggregate across `01-literals` + `10-bytes` and beyond.

## Gap-record output format (sexpr, not markdown)

The aggregate `wasm-opt-gaps.sexp` is a single top-level form: a header plus one
`(gap …)` record per case with a nonzero delta (zero-delta cases are optimal and
dropped), sorted by `o3` delta descending. One record per extracted core module
(a multi-module program contributes several, distinguished by `module`).

```
(wasm-opt-gaps
  (binaryen  "131")
  (from-trunk "b581d93e1")
  ; one (gap …) per case×module with a nonzero delta, biggest o3-delta first
  (gap
    (case   "spec/semantics/10-bytes.sexp" "the case title verbatim")
    (module 0)
    (size   (orig 273) (o3 232) (oz 232))
    (delta  (o3 41) (oz 41))            ; orig - opt, in bytes
    (metrics                            ; only categories that CHANGED, ours→opt
      (funcs 2 1) (Call 7 6) (imports 7 6) (LocalSet 6 7))
    (dominant  funcs)                   ; category with the largest DROP → ranks the gap kind
    (owner-lane inliner))               ; inliner | wasm-opt | core-opt | runtime
  …)
```

Field notes:
- `dominant` is the metrics category with the largest count DROP; it selects the
  gap KIND (per the taxonomy table) and thus the fix approach.
- `owner-lane` routes the row to the vertical that closes it — NOT every gap is
  `wasm-opt`'s. A `funcs`-dominated row (a forwarding wrapper inlined away) is
  `inliner` (v-compiler-ml / v-core-opt); a `LocalGet`/`LocalSet`/`vars`-dominated
  row is `wasm-opt` (this vertical). This column is what keeps prioritization
  honest — see the first-findings note below (the biggest deltas are `inliner`).
- Keep it flat and grep-friendly: an agent greps `(owner-lane wasm-opt)` for its
  own backlog and sorts records by `(delta (o3 N))` for priority.
- A case with NO `-O3` reduction is optimal on the primary signal, but `-Oz` (the
  size-only tier) may still shrink it. That is NOT dropped: it is emitted as
  `(optimal-o3 (case …) (module …) (size (orig N) (oz N)) (oz-delta N))` — a
  size-only opportunity the aggregator can list apart from the true `(gap …)`
  rows. Only when neither tier shrinks it is it a bare `(optimal (case …))`.

## Runnable target + per-case WAT diff

`v-nix` wires a nix target so any agent can run the analysis and inspect a case
without hand-driving the pipeline (surface, owned jointly — v-nix builds it,
this doc fixes its behavior):

- `nix run .#wasm-opt-gaps` — run over the whole corpus, write/refresh
  `wasm-opt-gaps.sexp`, print the ranked summary.
- `nix run .#wasm-opt-gaps -- --case <file-or-title>` — one case: print its
  `(gap …)` record.
- `nix run .#wasm-opt-gaps -- --case <…> --diff` — print the **WAT diff of OURS
  vs the wasm-opt output** for that case: `wasm-tools print` the extracted core
  module and the `wasm-opt --all-features -O3` output, then `diff` them. This is
  the "what did wasm-opt change vs what we wrote" view — it's how you go from a
  size number to the concrete emit pattern to fix (e.g. our `local.tee <fresh>;
  …; local.get <fresh>; local.set <dst>` vs wasm-opt's coalesced `local.tee
  <dst>`). Content-addressed like the aggregate, so re-running a case is free
  until its emit or binaryen changes.

The parse-and-format step is a dedicated, lightweight (std-only, zero-dependency)
program — the **`cdz-wasm-opt-gap`** crate (binary `wasm-opt-gap`,
`implementation/seed/crates/cdz-wasm-opt-gap/`) — NOT ad-hoc shell/awk over the
`--metrics` text (that is fragile: binaryen's aggregate `total` line trips a naive
parser). The per-case derivation runs `wasm-opt --all-features -O3/-Oz` +
`--metrics`, then invokes:

```
wasm-opt-gap --case NAME --module N --orig N --o3 N --oz N \
             --metrics-ours <ours.metrics> --metrics-opt <o3.metrics>
```

which parses both `--metrics` outputs, diffs them, and prints exactly the one
`(gap …)` record (or an `(optimal …)` marker) defined above. It runs no wasm-opt
itself, so it stays trivially cacheable/parallelizable. (A shell prototype of the
full pipeline — emit → unbundle → `-O3`/`-Oz` → record → WAT diff — also lives at
`claude-memory/repos/camshaft-cadenza/wasm-opt-gapscan-reference.sh` for local
hand-runs.)

## Mechanism architecture: per-case derivations + one aggregator (parallelizable)

The analysis is split into two derivation layers, mirroring how the corpus TEST
runs are structured — so Nix parallelizes the per-case work and caches each case
independently:

- **Per-case derivation** (`mkCorpusOptGap <case>`): one derivation PER case
  (per extracted core module). It takes that case's already-built `emit.wasm` as
  input, runs the pipeline (unbundle → `wasm-opt --all-features -O3`/`-Oz` →
  `--metrics`), and writes ONE per-case report: the `(gap …)` record (or an
  `(optimal (case …))` marker when the delta is zero). It is **content-addressed
  on `{emit.wasm, binaryen}`**, so a case re-runs ONLY when its emit or the tool
  changes — every other case stays cached. Because these are independent
  derivations with no cross-case dependency, Nix runs them **in parallel**
  (exactly like the per-case test derivations); the wasm-opt run is NEVER done
  inside the aggregation step.
- **Aggregator derivation** (`corpus-wasm-opt-gaps`): one final derivation that
  depends on ALL the per-case report derivations and does NOTHING but collect +
  order them — drop the `optimal` markers, sort the `(gap …)` records by o3-delta
  desc, wrap in the top-level `(wasm-opt-gaps (binaryen …) (from-trunk …) …)`
  form → `wasm-opt-gaps.sexp`. It runs no wasm-opt itself; it is pure reduction,
  so it is cheap and re-runs only when some per-case report changed.

This separation is the point: wasm-opt work is embarrassingly parallel and
per-case cacheable, while aggregation is a single serial reduce. Keeping them in
separate derivations (never a monolith that opts-then-aggregates in one build)
is what lets the corpus-wide sweep scale and incrementally update as this
backend improves.

## Workflow

- `v-nix` owns the flake mechanism: `mkCorpusOptGap` per-case over the already-
  built `emit.wasm`, content-addressed on `{emit.wasm, binaryen}`, aggregating to
  the generated `wasm-opt-gaps.sexp`; it is building the first slice
  (`01-literals` + `10-bytes`) and hands the populated doc to `v-wasm-opt`.
- `v-wasm-opt` owns this methodology, drives + prioritizes the aggregate doc
  (rank by size delta desc, group by gap kind), and CLOSES gaps with emit-side
  backend optimizations — each landed as a normal gated slice, re-measured to
  confirm the gap shrank without a behavior-corpus regression.
