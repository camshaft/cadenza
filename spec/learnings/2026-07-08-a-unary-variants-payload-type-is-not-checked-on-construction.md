# A unary variant's payload type is not checked on construction

*2026-07-08*

**What happened.** Adversarial probing of sum-type construction found that a unary variant with a
declared payload type does not check its argument against that type. `(type T (Mk Int64))` declares
`T.Mk : Int64 → T`, but `(T.Mk "x")` — applying it to a String — is accepted and constructs `(T.Mk "x")`,
an observably ill-typed value. The reverse (`(type T (Mk String))` applied to `42`) runs too. Multi-
variant sums have the same gap: `(type T (A Int64 | B String))`, `(T.A "wrong")` → `(T.A "wrong")`. The
`Ast` built-in sum shows it in the wild: `(Ast.Int "x")` → `(Ast.Int "x")` (Ast.Int's payload is Int64),
`(Ast.Name 42)` → `(Ast.Name 42)` (Ast.Name's is String). The mistyped payload is usable: matching
`(T.Mk "x")` binds the String, and a downstream `(String.byte-len n)` reads it as a String and succeeds
(running the ill-typed program); only a type-MISMATCHED downstream use — `(+ n 1)` — incidentally catches
it.

**Why it is a break.** core-semantics.md #A Sum Type Constructor Is A Single-Arity Function: a
constructor is "a single-arity function that, when applied to exactly one argument, produces a Sum value
tagged with the constructor's variant name" — and #Applying A Function Binds Its Parameter To Its
Argument type-checks the argument against the parameter. So `T.Mk : Int64 → T` applied to `"x"` is a type
mismatch, CDZ0201, exactly as `(f "x")` on an Int64-parameter `f` is. type-system.md #The Structural
Types makes a sum's shape "its variant names with their payload types", so a payload of the wrong type is
ill-typed. Constructing `(T.Mk "x")` is a false accept.

**Root cause (likely) — the constructor lowering checks the NULLARY (Unit) argument type but not a
declared non-Unit payload type.** The corpus already pins that a nullary variant's Unit argument is
checked (`(None 5)`, `(Sign.Pos 5)` → CDZ0201), so the constructor path has an argument check — but it
appears to special-case the nullary/Unit shape and not compare a unary variant's argument against its
declared payload type. So `(T.Mk "x")` is constructed with the argument as-is. The fix is to type-check
a constructor's argument against the variant's declared payload type for every variant — reusing the
ordinary function-application argument check — not only the nullary Unit case.

**The lesson (the recurring family).** An argument-type check landed for one variant shape (nullary →
Unit) but not carried to the sibling shape (unary → declared payload type), even though both are
single-arity function applications the spec types identically. This is the same "a check proven on one
form is not carried to its sibling" shape as the effect-operation argument-type gaps (c30/c48/c50 — where
the check landed for scalar parameters but not String/compound) — here on the ordinary sum-constructor
side rather than the effect-operation side. The tell: a nullary variant rejects a non-Unit argument, but
a unary variant accepts a payload of the wrong type. And it reaches the built-in `Ast` constructors, so a
self-hosted compiler could build a malformed `(Ast.Int "x")` node unchecked.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a unary variant applied to a wrong-type
payload is a type error" — `(T.Mk "x")` for `(type T (Mk Int64))` MUST reject CDZ0201, the typed-payload
companion of the nullary-variant cases (`(None 5)`, `(Sign.Pos 5)`) above it. Gated `(needs
sum-type-declaration)`, which the seed realizes, so the behavior gate runs and catches it (expected
reject CDZ0201, observed a running component constructing `(T.Mk "x")`). A generation that does not yet
check a unary variant's payload type declines rather than constructing the mistyped value.
