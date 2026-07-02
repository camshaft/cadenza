# Semantics Case Template

The executable-semantics corpus is a set of `NN-feature.sexp` files under `spec/semantics/`. Each
case is an s-expression in a small fixed test-DSL vocabulary, wrapping program fragments written in
Cadenza's canonical homoiconic representation. This form is parseable by a minimal s-expression
reader — the seed toolchain needs only that reader plus the reference interpreter to run the
behavior gate.

## The case vocabulary

```
(case "<short description>"
  (doc    "<optional prose for humans and agents; never affects the check>")
  (input  <program in the canonical representation>)
  (output (: <value> <Type>)))          ; the exact result execution must produce
```

Result clauses — a case carries exactly one of:
- `(output (: <value> <Type>))` — the value and type the program evaluates to.
- `(error <CODE>)` — the diagnostic code a program rejected at compile time must produce.
- `(trap "<reason>")` — the reason a program that halts at runtime must halt with.

## Example

```
(case "integer addition"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "no implicit promotion between numeric types"
  (doc    "Witnesses numeric-model.md #Numeric Types Do Not Silently Promote.")
  (input  (+ 2 2.0))
  (error  CDZ0301))
```

<!--
  AUTHORING RULES (delete before finalizing):
  - A case is EXECUTABLE and has exactly one definite result clause (output / error / trap);
    a case with no definite result is not a case.
  - A case covers ONE behavior; prefer many small cases so a behavior-gate failure names the
    construct that broke.
  - Output is byte-exact and deterministic; serialize the value under the canonical value form.
  - Every behavioral requirement in the corresponding capability spec is witnessed by at least one
    case here.
  - `input` fragments are the canonical representation itself — the corpus doubles as a corpus of
    real programs, with no second grammar to maintain.
-->
