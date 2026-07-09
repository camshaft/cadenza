## 35. ⚪ Polymorphic return-kind specialization (a function whose return kind is its argument's) — the `agree` follow-on to ask-34's decline

**Finding.** ask-34 (the `(id true)` → `1` miscompile) was resolved by DECLINING — `compiler.cdz` now traps on a
polymorphic function applied across the i64/i32 (Int/Bool) kind boundary rather than mis-widening the Bool to
i64. That eliminates the dangerous wrong-value miscompile (reject-don't-miscompile restored), but it is a decline,
not agreement: native compiles `(def (id x) x) (def (main) (id true))` to a valid `bool`-returning component,
`compiler.cdz` now traps. So the case is `decline` in the byte gate, not `agree`.

**The follow-on.** To reach byte-identity (`agree`), `compiler.cdz`'s return-kind machinery must **specialize a
pass-through return to the applied argument's kind** — a function whose body just returns a parameter has a
return kind equal to that parameter's kind at the call site, not the defaulted i64. The monotone return-kind
fixpoint (`build-ktab`/`ktab-iterate`) already propagates a BODY-shaped Bool return (a fn whose body is `(< a
b)` — those chains are byte-identical); this extends it to an ARGUMENT-shaped return: infer a kind variable for a
pass-through parameter and unify it with the argument kind at each call (monomorphize per call-site kind, as the
seed does for generics — see the host-value-agnostic monomorphization note).

**Priority.** LOW / deferred — the miscompile (the thing that mattered) is already gone via decline (ask-34
done). This is a completeness/coverage item: it moves polymorphic-identity-across-kinds from `decline` → `agree`,
narrowing the self-hosting frontier, but the compiler's own source uses few Bool-vs-Int-polymorphic pass-throughs
(most helpers are monomorphic or body-shaped), so it is not on the critical path. Revisit after the type-checker
(ask-30) and the discriminator (ask-33).

**Acceptance signal.** `compile-run <compiler.cdz>` on `(module m (def (id x) x) (def (main) (id true)))` returns
`true` byte-identical to native (moves `decline → agree`), instead of trapping.
Related: ask-34 (the decline that made this safe), the return-kind fixpoint learning
(`spec/learnings/2026-07-07-the-return-kind-table-is-a-monotone-fixpoint-and-it-propagates-bool-to-any-depth.md`).
