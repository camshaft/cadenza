# A flagged match-arm limitation was fixed — so the corpus workaround tightens to the precise pattern

*2026-07-07*

**What happened.** When `Ast.decode` became `Result<Ast, e>` (ask-38), the natural corpus form `(match
(Ast.decode …) ((Ok a) (= a x)) ((Err _) false))` was REJECTED by a seed limitation ("CDZ0201: comparison
between values of different types" on the leaf; "CDZ0401: undeclared capability" on a name in the arm), so the
cases shipped with an `(else …)` catch-all workaround and the limitation was flagged to the compiler agent. This
cycle the agent fixed it — probing confirmed the explicit `((Ok a) …) ((Err _) …)` arm now type-checks (round-trip
→ true) and nested `(Ok (Ast.Int n))` patterns work (→ 7). So the workaround is no longer needed, and the corpus
was tightened: the four decode cases that used `((Ok …) …) (else <error-value>)` now use the precise `((Err _)
<error-value>)` second arm. (The one genuine catch-all — "converts bytes to sum type," whose `else` covers `Err`
AND any non-`Ast.Int` `Ok` — correctly stays `else`.)

**Why.** `(else …)` and `((Err _) …)` are not equivalent, and the difference is what the case *pins*. An `else`
catch-all matches "anything not `Ok`-with-this-shape" — it passes whether the second variant is `Err`, some other
`Ok` shape, or a third variant that doesn't exist. The explicit `((Err _) …)` arm pins that the type is
*exactly* `Result` (`Ok | Err`) and that the error path is the `Err` variant — an exhaustive two-arm match that a
weaker `else` cannot express. Shipping the `else` form was the right call under the limitation (a passing case
now beats a precise-but-failing one), but it left the case under-specifying the surface; once the seed accepts
the exhaustive form, tightening to it is owed — the corpus should pin the *precise* contract, not the workaround
the seed forced. This is the mirror of the withheld-case discipline: a withheld case waits in an ask until the
seed can express it; a *shipped-with-a-workaround* case waits in the corpus and is tightened the moment the seed
removes the workaround's cause.

**The requirement it drove.** No new case and no new ask — this is a tightening of four existing decode cases
(`12-metaprogramming.sexp`) from `(else …)` to `((Err _) …)`, gate green (569). The loop closes its own Run-71
flag: I noted the limitation + workaround, the agent fixed it, and I verified the fix and upgraded the corpus in
the same channel. General lesson: **a workaround shipped to keep the gate green is a debt to the precision of the
spec, and the loop should carry it as a flag and pay it down when the seed removes the cause — a passing case
written around a limitation still under-pins the contract until it uses the shape the limitation forced it to
avoid.** (Separately, still owed by the compiler agent: `compiler.cdz`'s header comment "NOT YET: shifts `<< >>`"
remains stale — shifts landed last cycle; re-flagged.)
