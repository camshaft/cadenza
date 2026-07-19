; BREAKER FINDING 2026-07-18 (trunk 85e2cf5a1) — RUST-BACKEND Float32 LITERAL WIDTH family, TWO faces
; (the FLOAT sibling of the closed integer narrow-literal width family — same disease, float widths):
;
; FACE 1 — SILENT MISCOMPILE (always-false equality; COMPILES clean):
;   (def (main (: x Float32)) (if (= x 1.5) 1 0))
;   rust emit compares canonical BITS but the literal is emitted at F64 width:
;     ({x.to_bits() as u32}) == ({ f64::from_bits(4609434218613702656).to_bits() as u32 })
;   the RHS truncates the f64 bit pattern's LOW 32 bits — for 1.5 that is 0x0000_0000 — which can
;   NEVER equal the f32's bits (0x3fc00000). So `(= x 1.5)` is ALWAYS FALSE for every x:
;     rust: main(1.5) -> 0   main(2.5) -> 0        wasm: main(1.5) -> 1 (correct)
;   rustc accepts (u32==u32), so nothing catches it — silent wrong value on a differential.
;
; FACE 2 — COMPILE-FAIL (E0277/E0308): f32 literal ARITHMETIC emits the literal at f64:
;   (def (main (: x Float32)) (* x 2.0))  ->  (x * f64::from_bits(4611686018427387904u64))
;   E0277 "cannot multiply f32 by f64". Any Float32 param x float-literal arith fails rustc; the
;   f32-payload sum probe (FI.F/G with (* x 2.0) arms) hit E0308+E0277 the same way.
;
; wasm: BOTH faces correct (3.0; eq true), and f32 containers/sums/sets all verified clean this
; session — the bug is the RUST emit's float-literal WIDTH derivation: the literal rides in at its
; DEFAULT f64 form and no grounding to the operand's Float32 width happens on either the compare or
; the arith path. The integer twin of this bug (narrow int literal vs grounded value) was the
; five-member E0308 family v-rust-backend just CLOSED — this is the FLOAT column of the same table,
; and the fix shape is the same: derive the literal's emit width from the solved operand type
; (f32::from_bits(le 32-bit pattern)), both sides of compares, both operands of arith.
;
; SEVERITY: face 1 is a silent always-false guard on rust (a differential miscompile in any
; f32-literal-compare program); face 2 is a loud compile fail. Both blocked on the same width fix.
;
; Expected: (= x 1.5) at x=1.5 -> 1 (wasm's answer); (* x 2.0) at 1.5 -> 3.0.
(case "a Float32 parameter compares equal to a float literal at its own width"
  (doc    "`(= x 1.5)` with `x : Float32` — the literal must be emitted AT THE OPERAND'S width
           (f32), so x=1.5 compares equal (1) and x=2.5 does not (0), as wasm computes. The rust
           emit currently writes the literal as f64 and truncates its bit pattern's low 32 bits for
           the canonical-bits compare (0x0 for 1.5), making the equality ALWAYS false — a silent
           differential. The float column of the closed integer narrow-literal width family; same
           fix shape (ground the literal to the solved operand width).")
  (input  (do (def (main (: x Float32)) (if (= x 1.5) 1 0)) (export main)))
  (call   main (: 1.5 Float32))
  (output (: 1 Int64))
  (call   main (: 2.5 Float32))
  (output (: 0 Int64)))

; ---
; ROUTED to v-rust-backend (corpus-bugfix 2026-07-17): the FLOAT column of their closed 5-member integer
; narrow-literal width family. wasm correct (verified: (= x 1.5) -> 1). FACE1 SILENT: rust emits the
; Float32 literal at f64 width (low 32 bits of the f64 pattern = 0x0 for 1.5, vs f32 0x3fc00000) ->
; (= x 1.5) ALWAYS FALSE, main(1.5)->0 not 1 (breaker verified via rustc). FACE2 LOUD: (* x 2.0) -> E0277
; + E0308 in f32-payload sum arms. FIX (same as integer family): ground literal emit width to solved
; operand type (f32::from_bits) for compares AND arith. Not spawning (their family, fresh context). Promote when fixed.

; ---
; RESOLVED-PENDING-MERGE (v-rust-backend, 2026-07-18, commit 3fd71a7a5): both faces fixed via
; emit_grounded_float (grounds a ConstFloat operand to the op's float width), wired into FloatCompare +
; float emit_arith. FACE1: (= x 1.5) f32 now grounds to f32::from_bits(0x3fc00000), f(1.5)=1=wasm (was 0).
; FACE2: (* x 2.0) now x * f32::from_bits(...), g(3.0)=6=wasm (was E0277). f32-in-sum-payload emits now;
; Float64 unchanged. +1 pin, rust --check 0 regress. CLOSES THE WHOLE narrow-literal width family (all
; integer members + float). Committed on the pending host-closure S1 stack (4ec31ae41); sends after S1 lands.

; ---
; LANDED + VERIFIED (corpus-bugfix 2026-07-18, source-grep trunk c9940747e): emit_grounded_float defined
; (rust backend expr.rs:668) and wired into FloatCompare (2658-9) + float arith (1027-8) + branch (4451);
; f32 test present (tests.rs:3792/3828 "pick(b, x: f32) -> f32", grounds ConstFloat to f32 width). Both
; faces closed (silent (= x 1.5)-always-false + loud (* x 2.0) E0277). v-rust's host-closure S1 blocker
; cleared (stack now at S5 8aff7e373) so the f32 fix drained. Fully resolved.
