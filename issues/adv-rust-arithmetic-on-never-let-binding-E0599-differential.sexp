; BREAKER FINDING 2026-07-17 — rust-backend DIFFERENTIAL/miscompile: ARITHMETIC on a Never-typed (`!`)
; let-binding emits non-compiling Rust (error[E0599]: no method `checked_add` for type `!`).
;
;   (def (main) (let ((x (trap "boom"))) (+ x 1)))
;     x's initializer (trap "boom") is Never, so the binding diverges — the body (+ x 1) never runs.
;     Correct behavior: TRAP (unreachable) before the add. wasm does this.
;   cdz check -> PASSES rc=0.
;
; BACKEND SPLIT:
;   wasm       -> traps unreachable (CORRECT — the diverging init aborts before the body)
;   rust       -> artifact did NOT build: error[E0599] no method `checked_add` found for type `!`
;   rust-async -> same E0599
;
; NARROWED: the bug is ARITHMETIC (a method call) on the `!` binding, not the Never binding itself:
;   (let ((x (trap))) x)         -> rust PASSES (traps; no method call on x)
;   (let ((x (trap))) (+ x 1))   -> rust FAILS (emits `x.checked_add(1)` where x: `!`)
; rust binds `x: !` (never) then emits the body's `(+ x 1)` as `x.checked_add(1)` — but `!` has no
; inherent methods, so rustc rejects it. The dead arithmetic after a diverging init must not be emitted as
; a method call on `!`: rust should recognize the init diverges (emit the trap + `unreachable!()`/`return`,
; making the body dead) OR coerce `!` to the expected numeric type before the method call (`!` coerces to
; any type, so `let x: i64 = { trap }; x.checked_add(1)` would type-check — the binding's DECLARED type
; should be the body-expected numeric type, not `!`). wasm gets this right (the trap aborts; the add is
; unreachable and never emitted as a live op). backend/rust — the Never-binding lowering in mod.rs.
;
; NOT a wrong-VALUE miscompile (no bad number is produced — rust simply fails to build); a compile-vs-trap
; DIFFERENTIAL + invalid-Rust-emit. wasm is the reference (traps correctly).
(case "arithmetic on a let binding whose initializer diverges traps (does not emit a method call on Never)"
  (doc "(let ((x (trap))) (+ x 1)): x's init is Never, so it traps before the add. wasm traps unreachable
        (correct). rust+rust-async currently FAIL to build (E0599: no `checked_add` for `!`) — the dead
        arithmetic is emitted as a method call on the `!`-typed binding. Expected: trap unreachable on all
        3 (the diverging init aborts before the body; rust must not emit live arithmetic on `!`).")
  (input (do (def (main) (let ((x (trap "boom"))) (+ x 1))) (export main)))
  (trap  "unreachable"))

; SECOND WITNESS (breaker 2026-07-17): the SAME E0599 root fires via an inlined CALL ARGUMENT, not only a
; let-binding. (def (f (: x Int64)) (+ x 1)) (def (main) (f (trap "boom"))) — rust inlines f, substitutes
; the (trap) arg for x, producing <!>.checked_add(1) -> E0599. wasm traps unreachable (correct). So the bug
; is GENERAL: arithmetic on ANY Never value (let-bound OR inlined call-arg) emits a method call on `!`. The
; fix (coerce Never to the body-expected numeric type, or emit trap+dead-body) must cover both triggers.
(case "arithmetic reached via an inlined diverging call argument traps (not a method call on Never)"
  (doc "(f (trap)) with f x = (+ x 1): rust inlines f, substitutes the Never (trap) for x -> <!>.checked_add(1)
        -> E0599 (same root as the let-binding case above, different trigger). wasm traps unreachable. Expected:
        trap unreachable on all 3 once the Never-arithmetic emit is fixed.")
  (input (do (def (f (: x Int64)) (+ x 1)) (def (main) (f (trap "boom"))) (export main)))
  (trap  "unreachable"))

; ---
; UPDATE (corpus-bugfix 2026-07-17, from v-rust-backend note): FIXED in the same pending Never MR
; 8edddea3f. `(let ((x (trap))) (+ x 1))` now emits `panic!(unreachable)` (dead arithmetic on !, no
; checked_add on Never) — rustc-clean. Flips rust->pass once the MR lands. DO NOT mint a fixer.
