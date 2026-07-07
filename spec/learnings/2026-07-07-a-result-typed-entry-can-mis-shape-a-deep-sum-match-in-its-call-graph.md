# A result-typed entry can mis-shape a deep sum-match in its call graph

*2026-07-07. A conformance finding while wiring the diagnostics channel (ask-40's payoff) into the
self-hosted compiler. Descriptive — no RFC-2119 requirement; it records a shape-analysis subtlety and
why the durable response is the kinded-artifact interface (Amendment 0.8.0), not a point patch.*

## What happened

The diagnostics ABI (a `compile` body returning `Result<Bytes, list<diagnostic>>` is lifted as
`compile: list<u8> → result<list<u8>, list<diagnostic>>`) verified for *unconditional* bodies. But
turning the REAL self-hosted compiler's entry into a `Result` — so a rejected program returns a coded
diagnostic instead of trapping —

```
(def (compile b) (compile-result (resolve-module (read-module b))))
(def (compile-result funcs)
  (if (any-func-rejects? funcs (build-ktab funcs))   ; drives the deep Core sum-matchers
      (Err (reject-diagnostic))
      (Ok (compile-program funcs))))
```

made the compiler **decline itself**: *"runtime match with a non-literal pattern."* The exact same
`well-typed?` / `has-kerror?` sum-matchers (matching `Core` with nested `(tuple a b)` payloads) compile
fine when `compile` returns bare `Bytes`.

## Why (root cause, traced)

The trigger is a match-arm binder whose declared type is a compound. A `Func` is `(Fn (Tuple Int64
Core))`, and `func-body` returns the `Core` slot:

```
(def (func-body f) (match f ((Func.Fn (tuple np body)) body)))    ; body : Core  ⇒  Heap
```

Type inference does NOT seed a match arm's pattern binders with their declared payload slot kinds, so
when it infers the arm body `body` (a bare name not in the variable set), it reports "unknown" and
defaults the arm — and thus `func-body`'s return — to the scalar `Int64`, when it should be the heap
`Core`. That mis-inference is latent and harmless on its own. It becomes fatal through a cascade:

1. `func-body` returns `Int64` (should be `Heap`).
2. A caller `(well-typed? (func-body f) ktab)` sees an `Int64` argument where `well-typed?`'s parameter
   is `Heap` — a kind mismatch.
3. On a kind mismatch the compiler INLINES the callee (per-call monomorphization, the mechanism that
   makes polymorphism work in the coarse-kind seed): `well-typed?`'s body is emitted with its parameter
   *aliased to the `Int64`-typed argument node*.
4. Inside that inline, `well-typed?`'s own scrutinee is now a scalar, so its constructor-pattern
   `match` over `Core` falls to the SCALAR match path — which only handles int/bool literals — and
   declines "runtime match with a non-literal pattern."

So a single mis-inferred payload-slot kind, amplified by inline-on-kind-mismatch, drops a whole deep
sum-matcher onto the wrong lowering path. It surfaces only at the self-hosted compiler's scale/shape
(the FList/Func unpacking feeding the two deep matchers through the Result-shaped entry); a minimal
standalone reproduction of "extract a compound slot, then constructor-match it" does not trigger it.

## Why the durable response is the interface, not a point patch

The narrow inference fix — seed a match arm's constructor-pattern binders with their declared slot
kinds before inferring the arm body — is correct in principle but re-walks arm subtrees inside the
kind-inference fixpoint, which reintroduced a **compile-cost blowup** (the compiler went from
sub-second to >60s on itself — the same exponential-in-inference class a prior learning already fought).
A blowup-free version is possible but fiddly.

The deeper observation is that the mis-shape is provoked by the `result<Ok, Err>` entry specifically:
its two arms carry *different* payload types (`list<u8>` vs `list<diagnostic>`), and choosing between
them by a deep sum-match is where the analysis strains. The kinded-artifact interface (Amendment 0.8.0
— `compile: list<artifact> → record { artifacts, diagnostics }`) **avoids the strain by construction**:
success and rejection are ONE record type, so the `if`/`match` that chooses between "produced a
component" and "produced diagnostics" has same-shaped branches, and the deep sum-match in the condition
is an ordinary heap consumer. The same trigger body compiles cleanly under the record return. So the
interface generalization is not only more expressive (warnings, DWARF, multi-input) — it is also the
shape under which the compiler's own rejection logic lowers without a fragile analysis special case.

## The requirement it informs

No new normative requirement; this reinforces two existing ones. (1) Reject-don't-miscompile held
throughout — the mis-shape was always a *decline*, never a wrong component. (2) It is a data point for
the kinded-artifact interface (Amendment 0.8.0, build-tool-interface.md): a uniform result record is the
shape under which a self-hosted compiler reports diagnostics without the differently-typed-arm analysis
hazard. The point-fix at the inference layer stays a tracked seed ask (a match arm's binder should carry
its declared slot kind, implemented without re-walking the fixpoint), independent of the interface work.
