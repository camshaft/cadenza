;; ROOT-CAUSED (v-inference, one layer deeper than 'pre-fold snapshot'): it's the fold's
;; CAPTURE-AVOIDING FRESHEN pass, NOT resolve_bin. The bin operand ref resolves Unbound against the
;; MID-#renamed tree, so the rename is SKIPPED (the binder becomes #a but the ref stays 'a').
;; tuple/list operands dodge the ordering; bin's segment-nesting hits it. v-inference owns it,
;; dedicated-tick fix. (Perform-irrelevant: (def a (+ x 1)) in a handle body reproduces identically.)

;; REFINED (breaker, final): PERFORMING-NESS IRRELEVANT — any do-def in a HANDLE BODY unbinds in a
;; bin operand. (def a (+ x 1)) — NO perform — inside the handle body fed to (bin (u8 (UInt8.wrap a)))
;; = same CDZ0101; the no-handle twin computes 6. So the discriminator is HANDLE-BODY do-def × BIN
;; construction operand. COMPLETE PERIMETER: tuple/record/list/Set.of (11101) + slice-bound/unquote
;; (203) ALL resolve post-fold-rewrite — the BIN encoder is the SOLE operand position resolving
;; against a PRE-rewrite scope snapshot. SEAM = handler-fold's body rewrite vs bin's operand scope
;; (bin resolves too early). OWNER: v-inference / bin-lowering. (Perform not required — the pin below
;; uses a perform but a plain (def a (+ x 1)) reproduces identically; either is a valid witness.)

;; PERIMETER (breaker): tuple/record/list-literal/Set.of construction operands ALL keep the
;; performing-def binding (11101 across 4 projections) — ONLY the bin encoder loses it. So the fix
;; target is BIN's operand lowering exclusively: it resolves operands in a scope snapshot taken BEFORE
;; the handler fold rewrites the do-defs, while every other construction form re-resolves after.

;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-inference fixes F2. Origin: breaker
;; FINDING (issue 000000017688 F2), re-confirmed trunk f1ee5c564 (POST do-def-shadow fix 6566bff81 —
;; distinct). A performing do-def's binding is lost when consumed by a `bin` CONSTRUCTION operand
;; inside the handler body: (def a (Src.next)) (def frame (bin (u8 (UInt8.wrap a)))) → CDZ0101
;; 'unbound name a' at the bin operand (both backends). DISCRIMINATOR: the bin construction
;; specifically — the identical program with (Bytes.of (list (UInt8.wrap a))) instead of the bin
;; COMPUTES 10 (const twin works 1015). Resolve/scope interaction between the performing-def binding
;; and the bin-encode operand lowering. OWNER: v-inference (resolve) / bin-lowering operand scope.
;; ON FIX: gate x3 → 10; pin into 14-effects or 10-bytes (bin) beside the perform/bin pins; baseline x3.

(case "a performing def feeding a bin-construction operand stays bound (no false unbound)"
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (main (: x UInt8))
          (handle Src 10
            ((next (u) s (resume s (+ s x))))
            (do
              (def a (Src.next))
              (def frame (bin (u8 (UInt8.wrap a))))
              (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1)))))
        (export main)))
  (call   main (: 5 UInt8)) (output (: 10 Int64)))
