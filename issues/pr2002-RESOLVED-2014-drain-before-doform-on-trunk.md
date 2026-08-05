# PR #2002 review — rcdzc/src/effects.rs (v-effects) — MERGED — correctness [VERIFIED-PLAUSIBLE, HIGH-class]

https://github.com/camshaft/cadenza/pull/2002 (FIX inner-abort rolls back outer-effect advance — the
ao1-ao4 do-form abort fix). Copilot (id 3711423857) flags the new do-form early-return can skip the
`drain_and_wrap` of pending self-call temps.

## the do-form early-return (`return Some(rewritten)`) bypasses `drain_and_wrap(ctx, 0, rewritten)` → a `ctx.pending` multivalue temp referenced in the kept pre-abort foreign prefix stays unbound (Copilot, effects.rs:2477) — correctness [VERIFIED-PLAUSIBLE]
> This early-return path can skip draining `ctx.pending` (multi-value self-call temps) before returning
> `rewritten`. If threading created a temp (via `db.multivalue_specs` in `thread_bounded`) and that temp
> is referenced inside the kept pre-abort foreign prefix, returning without `drain_and_wrap` leaves an
> unbound temp name and can cause a spurious CDZ0101 / lowering failure.

VERIFIED the control flow on trunk. The new do-form abort return is:
```
if db.ast.as_form(rewritten, "do").is_some() && body_reaches_foreign_perform(db, body, &ctx) {
    reparent_under_handle_site(db, rewritten, body);
    return Some(rewritten);          // effects.rs:2474
}
```
The NORMAL path (the fall-through below it) does `let wrapped = drain_and_wrap(db, &ctx, 0, rewritten);`
(effects.rs:2497) — which pops every `ctx.pending` self-call temp and wraps `rewritten` in the binding
`let`s. `drain_and_wrap`'s own doc (:4198): "Pop every pending self-call temp … and wrap `inner` in one
`(let ((temp …)))` … Returns just `inner` when nothing was pending." So the do-form early-return at :2474
returns `rewritten` WITHOUT that drain. If `thread_bounded` (which threaded the kept pre-abort foreign
prefix) pushed a `ctx.pending` temp — a multivalue self-call in that prefix, e.g. `(do (relabel-self …)
(A.tick) (B.bail 99))` where the self-call arm pushed a `#t` temp + returned `(. t 0)` — that temp's
binding `let` is never emitted on the do-form path, leaving `(. t 0)` referencing an unbound `#t` →
CDZ0101 / "no machine representation" lowering decline. MED-HIGH (correctness, in a HIGH-class abort fix,
but reachability needs a multivalue self-call IN the kept foreign prefix of a do-form abort body).

Fix per Copilot: drain before the do-form return too —
`let drained = drain_and_wrap(db, &ctx, 0, rewritten); reparent_under_handle_site(db, drained, body);
return Some(drained);` (or hoist the `drain_and_wrap` above the do-form branch so both paths share it).
v-effects should confirm with a witness: a do-form abort body whose kept foreign prefix contains a
multivalue self-call (the `db.multivalue_specs` path) → check it doesn't emit an unbound temp. v-effects
owns effects.rs.
