# A unary constructor applied to zero arguments fabricates a unit payload

*2026-07-07*

**What happened.** Adversarial probing of constructor arity found a wrong-value miscompile at
the *low* end of the arity check. `(Some)` — the unary constructor `Some` applied to zero
arguments — evaluates to `(Some unit)`, a value of type `Option Unit` the program never wrote.
The same holds for `(Ok)` → `(Ok unit)`, `(Err)` → `(Err unit)`, and a user unary variant
`(B)` → `(B unit)`. The fabricated payload is observable: `(match (Some) ((Some x) 111) (_ 222))`
returns `111`, binding `x` to the fabricated `unit`. And it evades the payload-annotation check
the seed already enforces: `(: (Some unit) (Option Int64))` is correctly rejected (Unit ≠ Int64),
but `(: (Some) (Option Int64))` yields `(Some unit)` — an `Option Unit` value under an
`Option Int64` annotation, accepted and run.

**Why it is a break.** core-semantics.md #A Sum Type Constructor Is A Single-Arity Function:
a constructor "when applied to **exactly one argument**, produces a Sum value." `(Some)` supplies
zero arguments — the mirror of the over-application `(Some 1 2)` the corpus already rejects
(CDZ0201). A Unit filler is correct only for a *nullary* variant, whose argument type genuinely
IS Unit (`(None unit)`, `(Sign.Zero unit)`). A *unary* variant demands its one argument; supplying
none is an arity error, not a license to synthesize the payload. So `(Some)` MUST be rejected
(CDZ0201, as over-application is), not run to `(Some unit)`.

**Root cause — the constructor fold defaults a missing payload to unit unconditionally.** In the
seed (`codegen.rs::eval_const`, the `is_constructor_name(head)` arm), the payload is
`match items.get(1) { Some(p) => eval(p), None => CVal::unit() }`. The `None => CVal::unit()`
branch fires for *every* constructor applied to zero arguments, nullary or unary. It is right for
a nullary variant but fabricates a payload for a unary one. The compiler already tracks a
`nullary_variants` set (it uses it to *reject* a non-unit payload on a nullary variant); the fix
is to consult it here too — a zero-argument application of a name NOT in `nullary_variants` is
under-application, which must decline or reject (CDZ0201), not default to unit.

**The lesson.** An arity check written for the high end (over-application) leaves the low end
(under-application) to a default, and the default that reads naturally — "no payload node ⇒ unit
payload" — is exactly the nullary-variant rule applied indiscriminately. The nullary/unary
distinction is a *precondition* of the unit default, not a consequence of it: the same
`nullary_variants` set that rejects `(None 5)` must gate the unit filler, or the filler
manufactures a payload for the unary case. The tell was the annotation asymmetry — `(Some unit)`
rejected but `(Some)` accepted — which means the two took different construction paths for what
should be the same ill-typed value.

**Corpus case added.** `spec/semantics/09-functions.sexp` §"under-applying a unary constructor is
a type error, not a fabricated unit payload" — `(Some)` MUST reject CDZ0201, as the
under-application companion to the existing over-application cases. Native seed; the behavior gate
catches it (expected reject CDZ0201, observed a running component).
