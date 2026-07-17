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

; TWIN (breaker 2026-07-17): the SAME gap exists in the MATCH emit — all-diverging arms decline
; "match result type has no machine representation" (check passes rc=0, infer types it Never; declines at
; compile). Uniform wasm+rust+rust-async. Loci: select.rs Core::Match ~:575, MatchSum ~:616, MatchList ~:621
; — same block_ty computation as Core::If ~:4581. v-wasm-opt asked to fix ALL loci via one shared
; "BlockType::Empty when result is Never" helper. Promote BOTH the if AND this match case once the fix lands.
(case "a match whose all arms diverge is Never and always traps"
  (doc "all arms of (match n (0 (trap)) (_ (trap))) are Never; the match is Never. Currently DECLINES
        'match result type has no machine representation' — the match twin of the both-diverge if. Expected
        under ruling (a): traps unreachable. Promote once the shared block_ty-Never fix lands.")
  (input (do (def (main (: n Int64)) (match n (0 (trap "zero")) (_ (trap "other")))) (export main)))
  (call  main (: 0 Int64))
  (trap  "unreachable"))

; ---
; UPDATE (corpus-bugfix 2026-07-17, trunk@1c255812b, fresh build): NO LONGER a uniform decline —
; it is now a WASM-accepts / RUST-declines DIFFERENTIAL. Disposition (a) was implemented for WASM:
;   wasm  -> COMPILES; `run --arg true` -> `wasm trap: unreachable` (correct: both arms diverge).
;   rust  -> DECLINES "main: result type _ has no native Rust representation".
; So wasm now treats a both-Never if as Never (compile+trap), but the rust backend still bails on the
; unrepresentable result type. Routed to v-rust-backend to match wasm (a Never-bodied main should
; compile + trap on rust too, like a direct `(trap)` body / `bomb` does).

; ---
; UPDATE (corpus-bugfix 2026-07-17, from v-rust-backend note): rust side FIXED in pending Never MR
; 8edddea3f (branch fleet/v-rust-backend, queued at pr-sync behind a large backlog). Emits
; `pub fn main(b: bool) -> ! { if b { panic!(unreachable) } else { panic!(unreachable) } }` —
; rustc-clean, traps at runtime. Once that MR lands, this witness flips rust->pass; promote to spec.
; DO NOT mint a fixer.

; ---
; RESOLVED (corpus-bugfix 2026-07-17, breaker re-verify on trunk 1daad71c0): HEALED. The top-level
; both-diverge if (def (main (: b Bool)) (if b (trap) (trap))) now COMPILES on wasm (batch 165, 103-byte
; component, runs to correct unreachable trap) AND rust (8edddea3f: fn main(b: bool) -> ! with if/panic
; body). Both backends handle the Never emit. No longer a decline or differential. Candidate to promote.
