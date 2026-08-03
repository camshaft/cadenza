# PR #1528 review comments — rcdzc/src/lower.rs + spec/semantics/14-effects-and-handlers.sexp (v-effects)

Mirrored from https://github.com/camshaft/cadenza/pull/1528 (PR: "[v-effects] 7a5eaccd3").

## "A compiling `(host …)` block ALWAYS performs" over-claims (Copilot, lower.rs:8093 + 14-effects-and-handlers.sexp:6790) — doc/correctness
> [lower.rs:8093] The new comment asserts that a compiling `(host …)` block "ALWAYS performs" and
> therefore that `Resolved::Host` in the scrutinee implies a host perform. But the codebase already
> allows a well-formed host body with no perform (e.g. `(host (E) (E.get))` compiles, per tests.rs:
> 64310-64316). This makes the comment misleading; the check is still reasonable as a conservative
> over-approximation, but it should be described that way.
> [14-effects-and-handlers.sexp:6790] The case docstring repeats the "compiling host body always
> performs" claim — inaccurate for the same reason.

The `Resolved::Host` ⇒ host-perform check is fine as a CONSERVATIVE OVER-APPROXIMATION, but both the
lower.rs comment and the spec docstring state it as an invariant that doesn't hold — `(host (E)
(E.get))` (op-reference-only, no perform) compiles. Reword both to describe it as a conservative
over-approximation, not a guarantee that every compiling host block performs.
