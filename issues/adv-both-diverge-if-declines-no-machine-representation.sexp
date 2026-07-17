; BREAKER FINDING 2026-07-17 — an `if` whose BOTH branches diverge (Never) DECLINES with
; "if result type has no machine representation", uniformly on wasm+rust+rust-async (NOT a differential).
;
; CONTRAST that makes it look like a GAP, not by-design:
;   (def (main (: b Bool)) (trap "x"))            COMPILES (direct Never body — like `bomb` at 07-type:954)
;   (def (main (: b Bool)) (if b (trap) (trap)))  DECLINES "if result type has no machine representation"
;   (def (main (: b Bool)) (if b (bomb) (bomb)))  DECLINES (same) — both arms call a Never-returning fn
;
; The spec (type-system.md #Never Is The Empty Sum) says a diverging expression is Never and "Never unifies
; with any expected type". 07-type:942 pins that a ONE-diverging-branch if works: (if b 1 (trap)) is Int64
; (the Int64 then-arm gives the if a concrete machine type; Never unifies into it). 07-type:954 pins that a
; Never-BODIED FUNCTION (bomb) compiles + is callable in (+ 1 (bomb)). But a BOTH-Never `if` has NO concrete
; arm to take the machine type from, so the if-block wasm emit (which needs a concrete result type for the
; block) declines. Arguably it SHOULD compile: main's body is Never (both arms trap), exactly like `bomb`'s
; body is Never — a Never-bodied main should be callable + trap at runtime, not decline. The if-emit could
; special-case a both-Never if (emit either arm — both diverge — or an unreachable block).
;
; DISPOSITION (owner's call — v-inference owns the "if result type" resolution / Never unification):
;  (a) it SHOULD compile (both-Never if is Never, like a Never-bodied fn) -> fix the if-emit to handle a
;      both-Never result; then this becomes a passing (trap "unreachable") case. OR
;  (b) it's an intended decline (a bare Never if in value position has no representation) -> then the
;      DIAGNOSTIC should be clearer ("both branches of this if diverge; ..." not the internal-sounding
;      "if result type has no machine representation"), and pin the decline as a documented todo.
; NOT a miscompile (all 3 backends agree = decline). Reported for a ruling, not filed as a Fail.
(case "an if whose both branches diverge is Never and always traps"
  (doc "both arms of (if b (trap) (trap)) are Never; the if is Never. With b=true the then-branch traps.
        Currently DECLINES 'if result type has no machine representation' on all 3 backends — see header for
        the contrast (a direct Never body compiles; a Never-fn call in (+ 1 _) compiles). Expected under
        disposition (a): traps unreachable. Under (b): a documented decline with a clearer diagnostic.")
  (input (do (def (main (: b Bool)) (if b (trap "then") (trap "else"))) (export main)))
  (call  main (: true Bool))
  (trap  "unreachable"))
