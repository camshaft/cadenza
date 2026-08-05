# PR #2195 review — rcdzc/src/tests.rs (v-effects) — OPEN — comment/citation-accuracy [VERIFIED, LOW] (+ a diagnostic-semantics question)

https://github.com/camshaft/cadenza/pull/2195 (pin conditional-resume arm × perform-count — one-perform
folds, two declines cleanly, never miscompiles; breaker ob-family). Copilot 1 inline (2 sites).

## the pin's comment calls the two-perform decline "clean decline (CDZ0101)", but CDZ0101 is the UNBOUND-NAME diagnostic — an odd/misleading label for an unsupported-fold decline (Copilot, tests.rs:68177 & :68184) — comment-accuracy [VERIFIED, LOW]
> The comment says the current behavior is a clean decline "(CDZ0101)", but CDZ0101 denotes an
> unbound-name rejection rather than a decline. If the intent is to allow any clean decline in the current
> compiler, it's safer to avoid citing CDZ0101 here.

VERIFIED: `Code::Unbound => "CDZ0101"` (diag.rs:359) — CDZ0101 IS specifically the unbound-name diagnostic.
The #2195 pin comment (diff:34) says "Today this declines cleanly (CDZ0101)", and the assert (diff:42-44)
accepts `e.code.as_deref() == Some("CDZ0101") || e.code.is_none()`. So the comment labels an UNBOUND-NAME
code as "clean decline" for a two-perform conditional-resume face — misleading, since a "clean decline"
here should mean an uncoded "not-yet-foldable" decline, not an unbound-name rejection. LOW/comment-accuracy.
Fix per Copilot: don't cite CDZ0101 as the clean-decline code in the comment — say "declines cleanly (an
uncoded decline, or today CDZ0101)" or just "declines cleanly (no miscompile)".

NOTE (worth a v-effects glance, not a separate finding): the assert ACCEPTING `Some("CDZ0101")` implies the
current compiler may actually surface this two-perform decline AS an unbound-name error (CDZ0101). If so,
that's a slightly-wrong DIAGNOSTIC (an unbound-name code for what is really an unsupported-fold decline) —
benign for the pin's purpose (guarding against miscompile), but if v-effects expects an uncoded decline
here, the CDZ0101 arm is masking a mis-coded diagnostic. Contrast: this is the INVERSE of my #2176 finding
on the sibling cc-family pin (there the `Err(_) => {}` arm was TOO loose; here the arm is tight — specific
code or none — but the SPECIFIC code cited is a questionable one). Either way the comment shouldn't call
CDZ0101 a "clean decline". v-effects owns rcdzc effects + the breaker pins. PR OPEN → foldable pre-merge.
(Corpus/breaker-pin discipline: this is a lib-test in tests.rs, not a `.sexp` corpus case, so it's
v-effects' call whether the CDZ0101 arm stays or the decline should be uncoded.)
