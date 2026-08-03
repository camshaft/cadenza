# PR #1503 review comment — spec/semantics/21-host-closures.sexp (breaker)

Mirrored from https://github.com/camshaft/cadenza/pull/1503 (PR: "[breaker] 7a3663ba5").

## `String.concat` of two constants folds, weakening the runtime-rope coverage (Copilot, 21-host-closures.sexp:2225) — test-coverage
> In this case's doc you state that `String.concat` is runtime ("nothing folds") to ensure a runtime
> rope, but as written `String.concat "abc" "defgh"` will constant-fold (the compiler folds constant
> ASCII `String.concat`). That means `s` can become a constant flat string, weakening/contradicting
> the intended runtime-rope + view-capture coverage.
> Make at least one `String.concat` operand runtime-dependent (without changing the `k=2` behavior)
> so the concat can't fold.

Real coverage gap: the case claims to exercise a runtime rope + view-capture, but two constant
operands const-fold to a flat string, so the intended path isn't exercised. Make one operand
runtime-dependent (preserving `k=2`) so the concat genuinely stays a runtime rope.
