# The between-digits separator rule was fixed for integers but not floats

*2026-07-08*

**What happened.** After the digit-separator between-digits rule was fixed for INTEGER literals
(cycle 10: `1_`, `1__0` now reject CDZ0201), adversarial probing found the same rule is NOT
enforced for FLOAT literals. A `_` misplaced in a float — adjacent to the decimal point, trailing,
doubled, or stray in the exponent — is silently accepted with the `_` dropped: `1.5_` → `1.5`,
`1._5` → `1.5`, `1_.5` → `1.5`, `1.5__0` → `1.5`, `1.5e_10` → `1.5e10`, `1.5e10_` → `1.5e10`. The
integer forms of exactly these (`1_`, `1__0`) correctly reject; valid float separators between
digits (`1_000.5`, `1.2_5`) correctly work.

**Why it is a break.** A digit separator `_` is meaningful only BETWEEN two digits
(collections-and-text / the reader convention pinned in 01-literals.sexp), a both-sides condition —
the digit on each side must be present. A `_` adjacent to the `.` has a non-digit on one side; a
trailing `_` has no right-hand digit; a doubled `__` has a `_` on one side. All are malformed and,
being digit-led numeric tokens, are malformed literals (CDZ0201), not values with the `_` silently
stripped. The integer lexer now enforces this; the float lexer does not.

**Root cause — the fix was applied to the integer path, not the shared/float path.** The cycle-10
fix added between-digits validation to integer-literal lexing. Float-literal lexing still strips
every `_` from the token before parsing (the pre-fix behavior), so it never checks separator
placement in the integer part, the fractional part, or the exponent. The fix is to apply the same
between-digits check across a float token's three digit runs (integer, fraction, exponent), so a
`_` in any of them requires a digit on both sides — reusing the integer lexer's check rather than
leaving the float path on the old strip-all behavior.

**The lesson (the recurring family, at the reader).** A well-formedness rule fixed on one literal
form (integers) must be carried to every sibling form the rule covers (floats, and their fraction
and exponent digit runs). The between-digits rule is a property of a "run of digits with optional
separators," and a float has three such runs — the fix has to hold at each, not only in the integer
lexer where it was first written. This is the same shape as the collection-growth-operator and
if/match unselected-alternative findings: a rule enforced where it was first written, not at every
site that must maintain it. The tell: the identical malformed separator (`1_` vs `1.5_`) rejects as
an integer but is accepted as a float.

**Corpus case added.** `spec/semantics/01-literals.sexp` §"a digit separator adjacent to a float's
decimal point is a malformed literal" — `1._5` MUST reject CDZ0201, the float companion of the
integer `1_` case, with a note covering the trailing/doubled/exponent forms. Native seed reader; the
behavior gate catches it (expected reject CDZ0201, observed a running component that read it as 1.5).
Reader/front-door severity (like the integer case): a benign accepted value, but the same
malformed-literal class the corpus guards, and the between-digits fix was left incomplete for floats.
