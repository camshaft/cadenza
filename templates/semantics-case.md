# Semantics Case Template

The executable-semantics corpus is a set of `NN-feature.sexp` files under `spec/semantics/`. Each
case is an s-expression in a small fixed test-DSL vocabulary, wrapping program fragments written in
Cadenza's canonical homoiconic representation. This form is parseable by a minimal s-expression
reader — the seed toolchain needs only that reader plus the reference interpreter to run the
behavior gate. The corpus is **one flat set**: differences between generations (a typed front-end's
rejection; a capability a later generation realizes) are annotated *inline*, so a construct's meaning
lives in exactly one place.

## The case vocabulary

```
(case "<short description>"
  (doc    "<optional prose; never affects the check>")
  (input  <program in the canonical representation>)
  (output (: <value> <Type>)))          ; the primary result — what the interpreter (the oracle) does
```

**Primary result clause — exactly one; this is the oracle.** Usually a terminal clause (the outcome
of running the program):
- `(output (: <value> <Type>))` — the value and type the program evaluates to.
- `(trap "<reason>")` — the reason a program that halts at runtime halts with.
- `(exhausted)` — the program halts by exhausting the deterministic resource measure.

Or, for a program the interpreter's own front-end refuses *before* running it — a rejection needing no
type system (an unbound name, or an undeclared capability — the mandatory floor):
- `(error <CODE>)` — the diagnostic code (pinned registry) of that rejection, which *every* generation
  makes, including the dynamic seed.

**Observation clause — optional:**
- `(events (event <kind> <payload-value-form>)...)` — the exact ordered sequence of events the run
  emits; `(events)` asserts none. Part of observable behavior.

**Inline generation-divergence annotations — optional:**
- `(compiler (error <CODE>))` — a *typed* generation rejects this `input` at compile time instead of
  running it; a dynamic generation (the seed) ignores this clause and uses the primary clause. The
  compiler may diverge only by rejecting (if it runs, it must agree with the interpreter — the oracle).
- `(needs <capability>)` — the `input` needs `<capability>` to be evaluated at all; only a generation
  that realizes it runs the case. No `(needs …)` = core; every generation runs it.

## Example

```
(case "integer addition"                          ; core, agreed — no annotations
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "no silent promotion between numeric types" ; interpreter traps; typed compiler rejects
  (doc      "Witnesses numeric-model.md #Numeric Types Do Not Silently Promote.")
  (input    (+ 2 2.0))
  (trap     "numeric type mismatch")
  (compiler (error CDZ0301)))

(case "rational arithmetic is exact"              ; a later generation's capability
  (needs  numeric-model)
  (input  (+ (Rational.of 1 3) (Rational.of 1 6)))
  (output (: 1/2 Rational)))
```

<!--
  AUTHORING RULES (delete before finalizing):
  - A case is EXECUTABLE and has exactly one definite PRIMARY result clause — a terminal clause
    (output / trap / exhausted) or a front-end (error) — optionally plus (events ...), (compiler ...),
    and (needs ...). A case with no definite primary result is not a case.
  - The primary clause is the INTERPRETER's result — the one oracle. Do NOT write (compiler (output ...));
    the compiler may diverge only by rejecting.
  - A case covers ONE behavior; prefer many small cases so a behavior-gate failure names the
    construct that broke.
  - Output is byte-exact and deterministic; serialize the value under the canonical value form.
  - Every behavioral requirement of a capability the generation REALIZES is witnessed by at least one
    case that generation runs.
  - `input` fragments are the canonical representation itself — the corpus doubles as a corpus of
    real programs, with no second grammar to maintain.
-->
