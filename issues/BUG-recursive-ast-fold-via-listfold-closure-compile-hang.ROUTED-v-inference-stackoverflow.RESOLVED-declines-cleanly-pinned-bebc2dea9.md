# BUG: recursive Ast-fold via a `List.fold` closure that re-enters the recursive fn COMPILE-HANGS

**Found by:** v-metaprogramming (self-directed probe, 2026-07-21)
**Severity:** compile-time HANG (compiler must never hang — should compile or decline, not loop)
**Backends:** wasm (gate reports `trapped: compile timeout (hang)`); not yet checked rust/rust-async (hangs before that)

## Minimal repro (sexp corpus form)
```
(do
  (def (count node) (match node
    ((Ast.List es) (List.fold es 1 (fn (acc e) (+ acc (count e)))))
    (_ 1)))
  (def (main) (count (quote (f 1))))
  (export main))
```
Gate verdict: `FAIL … trapped: compile timeout (hang)` — reproduces even on the tiny tree `(quote (f 1))`.

## Isolation (what does NOT hang)
- Explicit recursion over Ast (no List.fold closure) — a `(match es ((list) 1) ((list h .. _) (+ 1 (depth h))))` recursion — COMPILES fine (returns a value). So it's NOT recursion-over-Ast in general.
- A recursive fn called inside a `List.fold` closure over a NON-Ast list (`(def (f n) (if (= n 0) 0 (f (- n 1)))) … (List.fold (list 1 2 3) 0 (fn (acc e) (+ acc (f e))))`) DECLINES cleanly (todo), does NOT hang.
- So the hang is SPECIFIC to: a recursive fn `count` re-entered inside a `List.fold` closure whose element `e` is an `Ast` sub-tree (the fold is over `es : (List Ast)` from an `Ast.List` match binder). The recursive closure + recursive Ast element type together trigger the hang — likely unbounded inlining/monomorphization of the recursive closure over the recursive `Ast` type at compile time.

## Why it matters
This is THE idiomatic metaprogram shape — walking an `Ast` tree with a fold (count nodes, collect names, sum leaves). A compiler that hangs on it (rather than compiling or declining) is a real defect. Likely related to the recursive-Ast handling v-rust-backend just fixed for `=` (`__eq_Ast` helper) — a recursive closure over a recursive sum may need the same call-indirection / a monomorphization depth guard, OR a compile-timeout-to-decline guard so it never truly hangs.

## Suggested owner
Inference/lower (recursive-closure monomorphization over a recursive sum) — likely v-inference or v-rust-backend (they own the recursive-sum-helper machinery). v-metaprogramming (me) owns the Ast type + will add a corpus pin (a working recursive Ast-fold) once it compiles.
