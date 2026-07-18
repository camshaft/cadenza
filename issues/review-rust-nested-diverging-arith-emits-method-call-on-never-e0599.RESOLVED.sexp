; REVIEWER FINDING 2026-07-17 — post-merge review of 8edddea3f (rcdzc rust-backend: "handle Never (`!`)
; in emit positions"). That fix closed the DIRECT diverging-operand case in `emit_arith`
; (backend/rust/expr.rs ~2549): if `body_diverges(lhs)` emit only lhs; if `body_diverges(rhs)` emit
; `{ let _ = <lhs>; <rhs> }`. But it MISSES the NESTED case — a Rust-backend miscompile (invalid emit) the
; green gate did not catch because no corpus case exercises nested diverging arithmetic.
;
; ROOT CAUSE: `body_diverges` (backend/wasm/select.rs:2857) recurses ONLY through If/Let/Seq/Match — its
; `_ => false` arm means a `Core::Arith` node is NEVER "diverging", even when one of ITS operands traps.
; (That is CORRECT for its wasm purpose — wasm `unreachable` is stack-polymorphic.) `lower_arith`
; (lower.rs:14118) propagates only `Core::Poison`, NOT `Core::Trap`, so `(+ (trap) 1)` stays a live
; `Core::Arith { lhs: <trap-node>, rhs: 1 }`, not a folded bare trap.
;
; So for `(+ (+ (trap) 1) 2)`: the OUTER emit_arith sees lhs = a `Core::Arith` (inner) → body_diverges=false,
; rhs = `2` → false. NEITHER new guard fires → the normal path emits `<emit inner>.checked_add(2)…`. The
; inner emit (its own lhs diverges) returns bare `panic!("unreachable")`, so the outer produces:
;
;   pub fn mk() -> i64 {
;       (panic!("unreachable")).checked_add((2u64 as i64)).unwrap_or_else(|| panic!("integer overflow in addition"))
;   }
;
; which is EXACTLY the E0599 the commit set out to eliminate — VERIFIED with rustc:
;   error[E0599]: no method named `checked_add` found for type `!` in the current scope
;
; So the "arithmetic on a diverging operand" family is NOT fully closed: it handles a diverging operand ONE
; level deep, but a diverging operand nested inside another arith (or any non-If/Let/Seq/Match value form
; that forwards a `!`) still emits a method call on Rust's `!`.
;
; SEVERITY: a Rust-backend emit that rustc REJECTS (E0599) — the rust/rust-async targets fail to build this
; program. wasm is fine (stack-polymorphic unreachable). NOT a wrong-value miscompile; it's an invalid-emit
; (compile-fails-on-the-target) gap, the same class the fix was closing. Low reachability (needs nested
; arith over a literal trap), but it is the direct residue of the just-landed fix.
;
; FIX SKETCH (v-rust-backend's call): the emit_arith guard needs to catch a diverging operand at ANY depth,
; not just a bare-diverging direct child. Options: (1) a rust-emit-local "does this operand's EMITTED Rust
; diverge / is it `!`-typed" check that recurses into Arith operands (mirroring what the direct guard does,
; but transitively); or (2) fold a diverging arith operand to `Core::Trap` at lower tier so `body_diverges`
; sees it (but that changes shared Core the wasm backend also reads — verify wasm parity first). The
; existing pin `a_diverging_body_or_operand_emits_never_not_a_decline_or_a_method_call_on_never`
; (backend/rust/tests.rs) should grow a nested case: `(+ (+ (trap "boom") 1) 2)` must emit only the trap and
; compile under rustc (no `.checked_add` on `!`).
;
; OWNER: v-rust-backend (author of 8edddea3f). Routed as a note to them + issue to corpus-bugfix.
(case "arithmetic nested over a diverging inner-arithmetic operand traps"
  (doc "(+ (+ (trap) 1) 2): the inner arith's lhs diverges, so the whole expression is dead — must emit only
        the trap (like the direct diverging-operand case), NOT `(panic!()).checked_add(2)` (E0599 on rustc:
        no method `checked_add` for type `!`). wasm is fine. Currently the rust/rust-async emit is invalid
        Rust — verified rustc E0599. Expected: emit only the trap; rustc accepts; the program traps at run.")
  (input (do (def (mk) (+ (+ (trap "boom") 1) 2)) (export mk)))
  (expect (trap "unreachable")))

; ---
; RESOLVED-PENDING-MERGE (corpus-bugfix 2026-07-17, per v-rust-backend note): FIXED in a15315393 —
; a transitive arith_operand_diverges predicate in emit_arith catches a diverging operand at any
; nesting depth (rust-emit-local; does NOT touch shared Core/wasm). (+ (+ (trap) 1) 2) now emits only
; panic!(unreachable), rustc-clean; pin grown with nested + rhs-nested cases. MR serialized behind
; v-rust-backend's pending sum-payload MR (same-file dependent stack) — sends once that lands.
; Verify rust-target PASS + promote once a15315393 integrates on trunk.

; LANDED (corpus-bugfix 2026-07-18): the arith_operand_diverges regression test is present on trunk (rust backend tests.rs) — the a15315393 transitive-diverging-operand fix landed. rust-target (owner-gate). Closed.
