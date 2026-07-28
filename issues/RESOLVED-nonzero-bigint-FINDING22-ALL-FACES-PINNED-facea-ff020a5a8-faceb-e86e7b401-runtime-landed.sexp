;; ============================================================================
;; HELD PIN (corpus-bugfix) — do NOT land until the wasm fix is on trunk.
;; Origin: breaker FINDING 2026-07-24 (inbox issue 000000016413-1894660).
;; A NONZERO BigInt-payload literal PATTERN probe inside a RECURSIVE function
;; miscompiles on wasm, two confirmed faces (both reproduced by corpus-bugfix
;; on trunk a1b15bb09):
;;   FACE A — silent WRONG VALUE: on the multi-variant Ast (Ast.Int payload =
;;     BigInt) the literal arm NEVER MATCHES; the match falls through to the
;;     catch-all, so a peephole rule `(* ,x 1) -> x` silently returns its input
;;     unchanged. VALID module, wrong behavior. gate: expect 40, wasm gives -1.
;;   FACE B — INVALID MODULE: the minimal plain user sum `(type W (Mk BigInt))`
;;     with the same recursive shape emits an invalid wasm component:
;;     "invalid component: failed to compile: wasm[0]::function[2]".
;; SCOPE (from breaker's 13-row matrix, spot-checked): BigInt-ONLY (Int64/String
;;   payloads fine), NONZERO-only (literal 0 matches — why every landed peephole /
;;   nested-quote pin passes, they all use 0), RECURSIVE-fn-only (non-recursive
;;   fine; wrapper/mutual recursion also breaks), quote-machinery-INDEPENDENT
;;   (hand-built input + hand-written pattern both repro). pattern-1-vs-input-0
;;   also fails => the literal is not merely zeroed; the probe COMPARE is broken
;;   under recursive specialization.
;; PERIMETER (breaker addendum #22, 2026-07-24): the plain-int-literal payload
;;   spelling ((Mk 1) / (Ast.Int 1)-via-quote) is the ONLY ACCEPTED way to write a
;;   BigInt literal probe — and it is exactly the broken one, so there is NO
;;   working spelling of a recursive BigInt-literal probe today (binder+= is the
;;   only route). Rejected/declined spellings (all honest, NOT bugs): bare BigInt
;;   scrutinee + plain lit 7 = CDZ0201 mismatch; bare BigInt + 7N = decline (non-
;;   scalar pattern); sum payload (Mk 7N) = decline (head not a variant ctor);
;;   Rational 1/2 in pattern = malformed-numeric-literal reject (=> #22 does NOT
;;   extend to Rational). BigInt literal CONSTRUCTION in a recursive body
;;   ((+ acc 7N) accumulator) computes fine => CONSTRUCTION is unaffected; this is
;;   pattern-PROBE-only. Fix target: plain-int-in-BigInt-slot probe lowering under
;;   recursive specialization.
;; RUST: FACE-A (Ast/quote) declines honestly (todo — "non-scalar literal-payload
;;   probe not rendered by the Rust backend"); FACE-B (plain sum) COMPILES on rust
;;   => differential candidate once wasm is fixed.
;; ROUTED: v-wasm-opt (wasm emit — BigInt literal materialization under recursive
;;   specialization), cc v-inference (if it turns out to be mono/specialization).
;; ROOT CAUSE (v-wasm-opt, ACCEPTED 2026-07-24 — their lane, fix is their next unit):
;;   NOT the match-arm probe. The WAT for FACE-B shows `main` calling `walk` with
;;   the SECOND arg `(Mk 1)` (a W ctor w/ BigInt payload) emitted as a RAW
;;   `i64.const 1` instead of a BigInt HEAP LEAF wrapped in the sum — so main
;;   passes an i64 where walk wants the i32 W handle → 'expected i32, found i64
;;   (at offset 0x124)', invalid component function[2]. i.e. a BigInt payload in a
;;   CONSTANT / recursive-specialized SUM CONSTRUCTION materializes as a raw
;;   i64.const, not a bigint-of-i64 heap leaf (payload handle should be i32).
;;   FACE-A (Ast.Int, multi-variant) is the SAME defect surviving as a never-true
;;   compare (valid module, wrong value) rather than a hard type mismatch. Fix =
;;   emit the BigInt leaf in the const-sum materialization path; a staged
;;   bigint-cmp probe fix then becomes correct too. KEEP HOLDING; v-wasm-opt pings
;;   on land. (their repro: /tmp/bigint-faceB-repro.sexp)
;; SPLIT (v-wasm-opt 2026-07-24): the fix is now TWO separate bugs on TWO owners:
;;   • FACE-A + all RUNTIME-scrutinee BigInt-probe rows = the PROBE COMPARE
;;     (bigint-cmp). Fixed by v-wasm-opt MR 5505b5010 (QUEUED). On its land,
;;     FACE-A flips 40 (was -1); runtime-scrutinee probes verified (k=1→40,
;;     k=2→-1; recursive walk over runtime (Mk (BigInt.of k))→40).
;;   • FACE-B (the minimal constant `(walk 1 (Mk 1))`) = a SEPARATE lower
;;     const-fold bug: the CONSTANT (Mk 1) call-arg materializes its BigInt
;;     payload as a raw i64 (not a bigint-of-i64 heap leaf). ROUTED to
;;     v-inference by v-wasm-opt; NOT fixed by 5505b5010. HOLD FACE-B.
;; ON LAND — STAGED, per owner:
;;   A. v-wasm-opt MR 5505b5010 on trunk → rebuild cdz; gate FACE-A x3 (wasm 40;
;;      rust/rust-async stay todo — honest decline, DO NOT flip to a value they
;;      can't produce) + add a RUNTIME-scrutinee probe case (k=2→-1 / walk over
;;      (Mk (BigInt.of k))→40, which rust may render → check). Land FACE-A +
;;      runtime rows into 20-structural-editing.sexp; HOLD FACE-B. Notify
;;      v-wasm-opt to confirm which rows go green; notify breaker (probe half).
;;   B. v-inference const-fold fix on trunk → rebuild; gate FACE-B x3 (wasm must
;;      give 40, valid module); land the FACE-B row beside FACE-A. Notify
;;      v-inference + breaker: recursive-nonzero-BigInt-probe corpus-closed on
;;      BOTH faces.
;;   Common: roundtrip + corpus_roundtrip + silent-omission sweep + --check
;;   0-regression x3 before each MR.
;; POST-5505b5010 VERIFICATION (corpus-bugfix, 2026-07-24, trunk 096c1652a, cdz rebuilt):
;;   • wasm RUNTIME-scrutinee probe = FIXED. `(match (Mk (BigInt.of k)) ((Mk 1) 40) (_ -1))` with k a
;;     runtime PARAM → 40 at k=1, -1 at k=2 (both recursive and non-recursive). ✓
;;   • wasm CONSTANT probe (FACE-A `(quote (* y 1))` + FACE-B `(walk 1 (Mk 1))`) STILL TRAPS invalid
;;     component — a CONSTANT (Mk 1)/(BigInt.of 1) folds to a const BigInt sum whose payload materializes
;;     as raw i64 = the FACE-B const-fold bug (v-inference's lane), NOT the probe compare. So FACE-A ⊆
;;     the const-fold bug too — BOTH held for v-inference. (Corrects the earlier "FACE-A flips on
;;     5505b5010" plan: FACE-A's scrutinee is constant, so it needs the const-fold fix, not the probe.)
;;   • NEW rust/rust-async DEFECT surfaced: a RUNTIME BigInt sum-payload literal probe BUILD-FAILS on
;;     rust — `error[E0605]: non-primitive cast: Big as i64` — even NON-recursive. The rust literal-probe
;;     compare emits `Big as i64` instead of a BigInt compare. This is a HARD build-fail (NOT an honest
;;     `todo` decline), so it cannot be baseline-tolerated. ROUTED to v-rust-backend separately. Until
;;     rust renders it, the runtime-probe pin can be wasm-only (rust row = the build-fail, HELD).
;; NET: nothing lands yet — the runtime-probe pin needs rust (routed); FACE-A/FACE-B need v-inference's
;;   const-fold. Re-verify on EITHER owner's land.
;; POST-77e8ca8b1 VERIFICATION (corpus-bugfix, trunk b9eb90e14, cdz rebuilt):
;;   • FACE-B (minimal plain-sum const `(walk 1 (Mk 1))`) = wasm NOW 40 ✓ (77e8ca8b1's const-fold fixed
;;     it). BUT rust + rust-async BUILD-FAIL `error[E0308]: mismatched types` — rust still can't render
;;     the CONSTANT BigInt probe (a DIFFERENT rust error than the runtime probe's E0605). HELD (build-fail
;;     ≠ baseline-tolerable). → route to v-rust-backend (const BigInt probe, sibling of the ecadf1221
;;     runtime fix).
;;   • FACE-A (multi-variant Ast `(quote (* y 1))`, quasiquote-pattern arm) = STILL TRAPS wasm
;;     invalid-component (function[17]) even post-77e8ca8b1. v-inference ACCEPTED + SHARPENED (their lane):
;;     it is NOT FACE-B's raw-i64-payload bug — the failure is 'unknown function 4294967295' = a call to
;;     func index -1 (u32::MAX sentinel). Isolation: (match (quote (* y 1)) ((Ast.Name _n) 40)(_ -1)) with
;;     NO simp = VALID (so quote/Ast.Int const-fold itself is FINE); simp on (quote y) [no *1 arm fires] =
;;     VALID; ONLY (simp (quote (* y 1))) where the quasiquote-pattern arm `(* ,x 1) FIRES + RECURSES
;;     (simp x) trips it. ⇒ ROOT: the recursive self-call inside a quasiquote-pattern arm that matched a
;;     const-folded quote emits an UNRESOLVED function index (-1 sentinel) under specialization/monomorph
;;     — a func-index-RESOLUTION bug in recursive-fn + quasiquote-pattern-arm interaction, NOT payload
;;     materialization. v-inference fixing on a dedicated tick w/ full battery; FACE-A HELD until then.
;;   • OWNERSHIP RESOLVED (v-inference CORRECTION 2026-07-24): FACE-A is fixed by v-wasm-opt's queued
;;     a2e7bea0d ("BigInt sum-payload literal probe collect must use the ENTERED variant, not variant 0")
;;     — NOT a separate v-inference commit. Root: collect_cont_ops_rec's LitTest import walk (select.rs
;;     ~3529) hardcoded variant_payload_ty_at(...,0) while emit resolves the entered variant via
;;     ty_at_path_recorded → import-set/CallImport-index divergence on a multi-variant BigInt-payload sum
;;     → the unknown-function-4294967295 (-1) index. v-inference independently built the byte-identical
;;     fix + DISCARDED it (v-wasm-opt's select.rs lane, got there first). FACE-A flips PASS on a2e7bea0d's
;;     land (QUEUED behind batch #126). So the WASM side of ALL faces closes with a2e7bea0d.
;;   • EMPIRICALLY SETTLED (v-inference 2026-07-24): cherry-picked a2e7bea0d onto trunk b9eb90e14 + ran
;;     the EXACT FACE-A repro (/tmp/facea-quote.sexp, the AST-quote simp matcher): TRUE TRUNK = func 17
;;     invalid (unknown function 4294967295); WITH a2e7bea0d alone = VALIDATES + runs → 40. So FACE-A and
;;     the multi-variant BigInt littest are the SAME root cause (collect's import-set walk resolving
;;     Payload via hardcoded variant-0 while emit uses the entered variant → shifted CallImport index →
;;     -1/u32::MAX func idx). There is NO separate 'recursive self-call func-index in quasiquote arm' bug
;;     — the -1 sentinel was the SYMPTOM of the shifted import index. a2e7bea0d closes BOTH FACE-A + the
;;     multi-variant littest. Only rust FACE-B E0308 (af3e8531f, queued) remains after a2e7bea0d lands.
;;   • runtime probe = wasm 40 ✓, rust E0605 FIXED by v-rust-backend ecadf1221 (probe compare Big-eq not
;;     as-i64) — QUEUED not landed.
;; RUST SIDE UPDATE (v-rust-backend, 2026-07-24): FACE-B's E0308 is a SEPARATE arm from the runtime E0605,
;;   also FIXED — commit af3e8531f (is_bigint_valued now STRIPS the nominal Ty::Nominal{inner:BigInt} of
;;   the erased newtype W, so the const (Mk 1) payload goes through const_big_expr not the int-literal
;;   path). Verified builds+runs rust+rust-async → 40. STACKED behind ecadf1221 (single-MR cadence, sends
;;   when FACE-A lands). So on af3e8531f+ecadf1221 landing, BOTH the runtime-probe row (FACE-A-shape) AND
;;   the FACE-B const row are rust-green; pin them cross-backend then. FACE-A (Ast-quote) still needs
;;   v-inference's func-index-resolution fix — the ONLY remaining blocker for the full matrix.
;; OWNER REFS: v-wasm-opt 5505b5010 (wasm probe, LANDED batch#124 PR#853 ✓) · v-inference 77e8ca8b1
;;   (FACE-A/B const-fold, is_bigint_valued nominal-peel, QUEUED not on trunk) · v-rust-backend ecadf1221
;;   (runtime probe E0605 fix — BigInt-eq compare instead of Big-as-i64 cast; verified builds+runs
;;   rust+rust-async 40/-1; QUEUED not on trunk, carries NO baseline flip — my pin adds the case).
;;   LAND the RUNTIME-PROBE case (below) cross-backend when ecadf1221 lands (wasm already 40/-1, rust
;;   confirmed by owner); LAND FACE-A/FACE-B when 77e8ca8b1 lands.
;; ============================================================================

;; RUNTIME-SCRUTINEE probe — wasm FIXED by 5505b5010 (40 match / -1 mismatch); rust build-fails E0605
;; (routed v-rust-backend). Non-recursive minimal + recursive form. Land cross-backend on rust fix.
(case "a runtime-built BigInt sum-payload literal probe matches its constructor (RUNTIME-scrutinee)"
  (input (do
        (type W (Mk BigInt))
        (def (main (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ (- 0 1))))
        (export main)))
  (call main (: 1 Int64))
  (output (: 40 Int64)))

(case "a runtime-built BigInt sum-payload literal probe falls through on mismatch (RUNTIME-scrutinee)"
  (input (do
        (type W (Mk BigInt))
        (def (main (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ (- 0 1))))
        (export main)))
  (call main (: 2 Int64))
  (output (: -1 Int64)))

(case "a nonzero BigInt literal probe in a RECURSIVE fn matches its own constructor (FACE-A repro)"
  (input (do
        (def (simp node)
          (match node
            (`(* ,x 1) (simp x))
            (other     other)))
        (def (main)
          (match (simp (quote (* y 1)))
            ((Ast.Name _n) 40)
            (_ -1)))
        (export main)))
  (output (: 40 Int64)))

(case "a plain-sum RECURSIVE nonzero BigInt literal probe emits a valid module (FACE-B repro)"
  (input (do
        (type W (Mk BigInt))
        (def (walk (: n Int64) (: w W))
          (if (< n 1) (- 0 1)
            (match w
              ((Mk 1) 40)
              (_ (walk (- n 1) w)))))
        (def (main) (walk 2 (Mk 1)))
        (export main)))
  (output (: 40 Int64)))
