; BREAKER FINDING 2026-07-20 (trunk 24381f9f3) — PIPELINE DIVERGENCE: the width-fit check is
; BACKEND-DEPENDENT. The same program over UInt8 params:
;
;   (+ (* 10000 (if (< a b) 1 0)) (% a b))
;
;   wasm target: compile-REJECTED — CDZ0302 "integer literal does not fit its width"
;                (width inference reconciles the literal 10000 into the UInt8-width expression,
;                 10000 > 255 -> reject; consistent with the pinned narrow-literal width rules)
;   rust target: COMPILES — emitting a silent TRUNCATING cast:
;                (((10000u64 as i64)).wrapping_mul(...) as u8)   <- `as u8` truncates 10000 to 16
;                then a runtime trap "integer overflow in multiplication" on some inputs / wrong
;                arithmetic on others. The 3-term variant ran and TRAPPED where wasm never compiles.
;
; One of the two must be wrong. Per the corpus width rules (a narrow param + bare literal computes AT
; THE PARAMETER WIDTH; a literal that does not fit its width is a coded reject: 06-numeric:3531/3791
; family + the CDZ0302/CDZ0201 literal-range rejects), the WASM REJECT is the conforming behavior —
; the rust path is missing the width-fit validation and instead emits `as u8` truncation, changing
; semantics silently (10000 -> 16) before the checked ops even run.
;
; ISOLATION:
;   (* 100 (/ a b))            both compile (100 fits u8)                          OK
;   (* 10000 (if (< a b) 1 0)) ALONE: both compile — the bare (* 10000 if) infers Int64?  OK
;   (+ (* 10000 if) (% a b))   wasm CDZ0302 / rust compiles-with-as-u8-cast        ✗ DIVERGENT
;   (+ 300 (% a b))            BOTH reject (CDZ0201 out-of-range for UInt8)        consistent
; So the divergence is the INFERRED-width path (the + with a UInt8 operand pulls the multiply into
; u8 width): the direct-literal check (300) is shared/frontend, but the post-unification width-FIT
; check fires only in the wasm backend; rust emits a truncating cast instead.
;
; SEVERITY: silent semantic change on rust (a wrong-value class, not just a loud trap — the `as u8`
; truncation runs BEFORE the checked add), and a well-formedness disagreement between pipelines: a
; program in the corpus could be baselined `(error CDZ0302)` on wasm and pass-with-wrong-value on
; rust. The gate's per-target baselines would mask it.
;
; Expected: BOTH targets reject CDZ0302 (the conforming outcome), OR both compute with agreed
; semantics if the width ruling is revisited — but never a silent truncating cast.
(case "an inferred-width literal that does not fit rejects identically on both targets"
  (doc    "`(+ (* 10000 (if (< a b) 1 0)) (% a b))` over UInt8 a/b: width unification pulls the
           multiply into the UInt8-width `+`, where the literal 10000 does not fit — a coded reject
           (CDZ0302), as the wasm target gives. The rust target must NOT instead emit a silent
           truncating `as u8` cast (10000 -> 16) — same program, same frontend, same verdict.")
  (input  (do
            (def (main (: a UInt8) (: b UInt8))
              (+ (* 10000 (if (< a b) 1 0))
                 (% a b)))
            (export main)))
  (error  CDZ0302))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk 24381f9f3) — VERIFIED, routed v-rust-backend (cc v-inference) =====
; CONFIRMED silent wrong-value: wasm rejects CDZ0302; rust COMPILES (0 build errors) emitting "10000u64 as i64"
; ... "as u8" -> truncates 10000->16 before checked ops -> wrong arithmetic. Well-formedness DISAGREEMENT.
; Trigger = INFERRED-width path (unification pulls the * into u8 via the +); direct literal rejects on both.
; Fix: rust emit must apply the same post-unification width-fit reject (CDZ0302), not a truncating as-u8 cast;
; OR (cleaner, cc v-inference) move the width-fit check to the SHARED width-reconciliation layer so both
; backends inherit it. NOT corpus-pinnable as-is (pass-wrong on rust, per-target baselines mask it) — pin once
; the reject is consistent across backends. Severity: silent wrong-value on rust. Match-spec 06-numeric:3531/3791.

; ===== OWNER FIXED at SHARED layer (v-inference, 2026-07-20) — MR f5efd9397 queued =====
; My routed locus question answered: YES, fixed at the SHARED width-reconciliation layer (both backends
; inherit the reject). Root matched triage exactly: the shared collect check only inspected a literal's
; IMMEDIATE binop sibling, so the TRANSITIVE-width path (10000's sibling is the deferred `if`; the `*`'s
; sibling under the `+` is the UInt8 (% a b)) slipped through -> check passed, wasm rejected CDZ0302, rust
; truncated as-u8. Fix climbs the integer-arith spine (arith op unifies operand widths; a comparison breaks
; the chain). All 3 (check+wasm+rust) now agree.
; ⚠ PIN CODE CORRECTION: the consistent verdict is CDZ0201 (CONTEXTUAL — a bare literal fixed by an
; integer-operand context is 'out of range for <T>', no annotation to blame), NOT CDZ0302 (which is the
; ANNOTATED (: 10000 UInt8) path). So when f5efd9397 LANDS, pin as (error CDZ0201), NOT (error CDZ0302).
; Both compile-time rejects per numeric-model.md (annotated=0302 vs contextual=0201). v-inference added an
; rcdzc unit test (4 cases incl 2 no-over-rejection guards) so the fix is gate-protected regardless.
; corpus-bugfix: on f5efd9397 land, re-pin this as (error CDZ0201) all-backends-consistent.
