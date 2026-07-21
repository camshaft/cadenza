; adv repro (found by v-verification while probing @ensures-over-handle-body composition, 2026-07-20)
;
; BUG: a `let` whose INIT is a `handle` expression, in a callee's body, makes the CALLER's runtime
; argument spuriously CDZ0101 "unbound name". The unbound name is the CALLER's parameter (`k`), not
; anything in the callee — so this is a name-resolution / effects-lowering interaction, NOT a verification
; bug. It surfaced through v-verification's `@ensures` enforcement (which injects `(let ((ret BODY)) …)`),
; but reproduces with a HAND-WRITTEN `let`-over-handle and no annotation at all, so it is a lower-layer bug.
;
; ISOLATION (each row = one program; ✅ compiles, ❌ CDZ0101 `unbound name k`):
;   ✅  main const-arg  + callee `(let ((r (handle …))) r)`      — n2e: (def (main) (f 5))
;   ✅  main typed-arg  + callee handle DIRECTLY the body (no let) — g1: (def (f x) (handle …))
;   ❌  main typed-arg  + callee `(let ((r (handle …))) r)`       — THIS CASE
; So the trigger is the CONJUNCTION: (a) caller passes a runtime arg `(f k)` where `k` is a typed param,
; AND (b) the callee binds the result of a `handle` with a `let` and returns the bound var. Drop either the
; `let` (put the handle directly in body) or the runtime arg (use a constant) and it compiles + runs.
;
; The `let`-over-handle shape is EXACTLY what `verify_enforce` emits for `@ensures` on an effect-handling
; def, so this blocks `@ensures`/`@requires` (which also wraps the body) over any def whose body is a
; handle expression when called with a runtime argument. Reported to v-effects/v-inference via the PM.
; Likely owner: effects lowering or name-resolution (the caller-arg binding is dropped when the callee's
; body contains a let-bound handle init). NOT verification's lane.

(case "a let-bound handle-init in a callee's body spuriously makes the caller's runtime argument unbound (CDZ0101)"
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (f (: x Int64))
              (let ((r (handle St x ((tick (u) s (resume s (+ s 1)))) (St.tick))))
                r))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk e371e1d50) — VERIFIED, routed v-effects (cc v-inference) =====
; CONFIRMED live: the exact repro fails cdz-compile CDZ0101 "unbound name k" (caller param), cdz check CLEAN.
; Isolation re-verified: const-arg (f 5) COMPILES(5); handle-direct-body typed-arg COMPILES(5); only the
; conjunction [runtime-arg call + callee let-over-handle-init] fails. Same caller-arg-drop CLASS as the
; closure-payload β-copy (6e6d45b20 pin) + same-name-ctor β-copy (b42821408), but trigger is let-over-handle.
; Blocks @ensures/@requires over handle bodies (verify_enforce injects (let ((ret BODY)) …)). Routed v-effects
; (effects-lowering) primary, v-inference cc (resolution/β-copy angle). corpus-bugfix pins it (compiles+runs 5)
; once fixed — likely all-3-backend green like the other β-copy caller-arg-drop pin.

; SHARPENED (v-effects, 2026-07-20 investigation — corrects the locus):
; - The CDZ0101 fires BEFORE handle-lowering: a VEFF trace at lower.rs's Resolved::Handle arm shows
;   reduce_handle is NEVER REACHED for this repro. So it is NOT a reduce_handle fold bug — the `k` is
;   dropped EARLIER, during the (f k) call's processing (the inline/β-reduce of f's let-over-handle body).
; - ISOLATION (all confirmed):
;     no-let, handle-seed = runtime arg (f k)        → COMPILES + runs 5   (handle alone is fine)
;     let-bound value = (+ x 1), NO handle, (f k)     → COMPILES + runs 6   (let-in-inlined-callee alone is fine)
;     let-bound handle whose SEED = x, (f k)          → CDZ0101 unbound k   ← the bug (let + handle-runtime-seed)
;     let-bound handle, x in BODY (not seed), (f k)   → COMPILES           (only the SEED position drops)
;     f called TWICE (+ (f k) (f (+ k 1)))            → still CDZ0101       (not simple single-inline)
; - So the TRIGGER is precisely: [a `let` binding a handle whose SEED references a caller runtime arg] in a
;   callee body, processed at the call site. A handle-in-body without the let, or a let without a handle,
;   both work — only the conjunction drops the seed's caller-arg binding, and it happens in the f-inline /
;   effects-pre-reduction (reduce_applied_lambdas or the general apply β-reduce), NOT reduce_handle.
; - A single deep_fresh_copy of `init` inside reduce_handle did NOT fix it (consistent — reduce_handle isn't
;   even reached). The fix is at the f-body-inline site that copies a let-over-handle and loses the seed's
;   free var. LIKELY the effects-pre-reduction copies f's body (handle present → effects path) with a
;   copy that doesn't pin/preserve the seed's caller-arg. NEXT: trace reduce_applied_lambdas / the apply
;   β-reduce of a callee whose body is (let ((r (handle …seed=arg…))) r).
;
; RE-SHARPENED + TRACE-BACKED (v-effects, 2026-07-21 — CORRECTS the "reduce_handle unreached" claim above):
; A VEFF eprintln trace in eval::beta_reduce + copy_structural on the EXACT repro proves reduce_handle IS
; reached and DOES fold (the pure-one-hole `resume`-rewrite path, effects.rs ~2050/2107 fires). The real
; mechanism:
;   1. (f k) inlines f's body: beta_reduce substitutes the handle SEED x -> the arg node carrying `k`
;      (trace: `SUBST ref name="x" -> arg=6054`). So far correct.
;   2. reduce_handle's pure-one-hole fold does `subst.insert(arm.state /*s*/, init)` (effects.rs:2077),
;      where init is that SAME seed node 6054. beta_reduce then splices it at EVERY `s` reference via the
;      direct `return arg` branch (eval.rs:536-537) — arm body `(resume s (+ s 1))` has TWO `s` sites, both
;      get node 6054 (trace: `SUBST ref name="s" node=6066 -> arg=6054` AND `... node=6068 -> arg=6054`).
;   3. A single node has ONE parent; push_list re-parents 6054 to the LAST `s` site, ORPHANING the earlier
;      splice. The `k` free var inside the orphaned copy re-resolves against no scope -> CDZ0101 unbound `k`.
; WHY THE ISOLATION MATCHES: const seed (f 5) = a constant leaf is scope-independent (shared safely, no
;   re-resolve); handle-direct-body has no `let`-routing of x into a foldable seed the same way; a
;   SINGLE-`s` arm would splice once (no orphan). The bug needs [runtime-arg seed] × [arm body uses `s` ≥2×].
; FIX TESTED-NEGATIVE: `resolve::resolve_subtree(db, init)` (pin the seed's free vars) BEFORE the subst.insert
;   does NOT fix it — the direct `return arg` substitution branch bypasses the pinned-share arm, so both `s`
;   sites still share the one node. Pinning alone is insufficient.
; FIX DIRECTION (for landing): the multi-`s`-use seed must not be a SHARED node. Either (a) in reduce_handle,
;   let-bind `init` ONCE at the fold site — the eval-once pattern apply_lambda already uses for multi-use
;   args (eval.rs ~1042-1080): substitute a fresh #s local at each site + wrap `(let ((#s init)) folded)`;
;   or (b) give beta_reduce a per-site fresh+pinned copy for a substituted arg used ≥2× whose subtree has a
;   free (caller-scope) name. (a) is the more localized fix (touches only the effects fold, and the seed is
;   pure so a plain let is sound). Gated by: this repro compiles+runs 5, and no gate/E5 fold regression.
; NOTE: v-effects has this queued — build is BLOCKED this tick on an in-flight unrelated MR (can't commit /
;   re-sync under a pending --ref). Fix lands the tick after that MR clears.
;
; DEEPER ROOT CAUSE + WORKING FIX FOUND (v-effects, 2026-07-21, 2nd trace tick — CORRECTS the fold-path locus):
; The failing fold is NOT the pure-one-hole block — it is the TAIL-RESUMPTIVE `thread` path. `(resume s (+ s
; 1))` is a tail-resumptive arm, so `reduce_handle` routes it to `thread` (effects.rs ~2331 `thread(db, body,
; vec![init], &ctx)`), which at the perform arm (effects.rs ~4036) does `subst.insert(arm.state, cur[slot])`
; (cur[slot] starts = init = the seed `(: k Int64)` node), β-reduces the arm body, then `peel_resume_from_arm_
; body` extracts value=`s`(→the seed) + next_state=`(+ s 1)`, and DEEP_FRESH_COPYs each (effects.rs ~4092-93).
; TRACE PROOF: instrumented the thread arm — value node = `List[":", k, "Int64"]` (the substituted seed wrap),
; and the `k` leaf inside IS resolve-PINNED (`resolved_subtrees.contains` = true, its resolution to main's
; param `k` memoized). deep_fresh_copy re-pushes it as a BARE fresh atom with an EMPTY `resolved` slot → the
; copy re-resolves against the folded ORPHAN (parent None) → unbound `k`. So the drop site is deep_fresh_copy
; DESTROYING the pin, exactly the "deep_fresh_copy is naive" hypothesis — CONFIRMED at the node level.
; FIX THAT WORKS (repro COMPILED OK): make deep_fresh_copy PIN-PRESERVING — when copying a node in
; resolved_subtrees, copy its memoized `Resolved` onto the fresh id (db.resolved.fill) + re-insert into
; resolved_subtrees. The copy is a distinct node (still breaks value/next-state sharing) that keeps the
; capture's resolution.
; BUT TOO BROAD — REGRESSES 3 existing tests (an_effectful_helper_in_a_selfcall_arg_folds +2): those NEED the
; fresh copy to RE-resolve against the SPECIALIZED def's sig (an INTERNAL state param `fuel`/`$s{k}` whose
; binder is being re-created in the copy), and blanket pin-preservation keeps their stale resolution → "unbound
; fuel". So the copy must preserve the pin ONLY for a capture bound OUTSIDE the whole fold (like `k`), NOT for
; an internal param re-bound by the copy. This is EXACTLY beta_reduce's existing scrutinee_is_substituted /
; `is_within(reduction_root)` discrimination (eval.rs ~574-593). REFINED FIX (next tick): gate the pin-
; preservation on the pinned node's binder being OUTSIDE the fold root (a genuine capture), else fall to the
; existing fresh-re-resolve. Needs the fold's root id threaded to deep_fresh_copy (or a `reduction_root`-style
; check). Then: repro compiles+RUNS 5, the 3 selfcall-arg tests stay green, full effects suite + gate green.
