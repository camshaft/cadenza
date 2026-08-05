# PR #2014 review — rcdzc/src/effects.rs (v-effects) — MERGED — correctness [VERIFIED-PLAUSIBLE] (follow-on to my #2002)

https://github.com/camshaft/cadenza/pull/2014 (drain pending multivalue temps before the do-form abort
return — the fix-forward for MY #2002 do-form drain-skip). Copilot (id 3711903042) flags the drain I
prompted is too broad: `mark=0` binds ALL pending temps, including dead-suffix ones.

## `drain_and_wrap(db, &ctx, 0, rewritten)` drains ALL pending temps (mark 0), including temps created while threading DEAD code after the abort fired → wrapping `let`s force-evaluate self-calls that shouldn't run (Copilot, effects.rs:2486) — correctness [VERIFIED-PLAUSIBLE]
> `drain_and_wrap(db, &ctx, 0, rewritten)` will bind/evaluate *all* pending multivalue temps, even if some
> were created while threading unreachable code after an abort fired (threading continues after
> `ctx.abort_value` is set). In the do-form abort early-return path this can change semantics by evaluating
> dead self-calls that should not run. Consider draining only the pending temps whose names are actually
> referenced in the returned `rewritten` (or in the inits of other kept temps)…

VERIFIED the mechanism, reachability UNCONFIRMED. This is a deeper concern on the exact fix I prompted in
#2002: the #2002 fix (effects.rs:2484) does `drain_and_wrap(db, &ctx, 0, rewritten)` with `mark = 0`, so it
drains EVERY entry in `ctx.pending` and wraps `rewritten` in a `let` for each. A `let`-bound self-call temp
is EVALUATED when its `let` runs — so if threading continued past the fired abort and pushed temps for a
DEAD suffix (self-calls after the abort that never execute at runtime), binding them in wrapping `let`s
forces those dead self-calls to run — a semantic change (the abort should have discarded them).

The hinge (v-effects' to judge): does threading actually CONTINUE after `ctx.abort_value`/the abort fires,
pushing pending temps for the dead suffix? The #2002 fix's own comment frames drain-at-0 as "no-op when
nothing pending (the common case)" — so in the common case this is moot; the concern is the tail where an
abort body's dead suffix contains a multivalue self-call. If threading short-circuits at the abort (pushes
nothing for the dead suffix), Copilot's concern is unreachable and drain-at-0 is fine. If it threads on,
the fix should drain only temps REFERENCED in `rewritten` (+ transitively in kept temps' inits) — a
referenced-set drain, not `mark 0`. VERIFIED-PLAUSIBLE, MED (correctness, in the actively-extended abort
path). Recommend v-effects confirm whether post-abort threading pushes temps, with a witness: a do-form
abort whose DEAD suffix (after the abort) holds a multivalue self-call → check the wrapping `let` doesn't
force it. (Composes directly with my #2002 — same code, one level deeper.) v-effects owns effects.rs.
