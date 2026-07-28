;; HELD PIN (corpus-bugfix, 2026-07-27) — do NOT land until v-wasm-opt fixes the invalid-module emit.
;; Origin: breaker FINDING (inbox issue 000000017047). CONFIRMED reproduces on trunk d17e0a484
;; (fresh build): String.to-bytes of a RUNTIME rope (String.concat-built), with the Bytes result
;; used across FOUR+ branch arms, compiles a module that FAILS wasm validation:
;;   cdz compile → 'wrote ... (1649 bytes)' but wasm-tools validate → 'func 12 failed to validate'
;;   cdz run     → 'invalid component: failed to compile: wasm[0]::function[12]'
;;   breaker's wasm-tools read: 'type mismatch: expected i32, found i64'.
;; DISCRIMINATOR (breaker matrix): 4+ uses of the to-bytes result across arms = FAIL; 3 uses always
;; OK (len+at0+at2, even with a seam-crossing slice view in the chain); const-foldable ropes fold
;; away and mask it; a NON-rope runtime string folds (no non-folding non-rope repro found — the
;; rope may be required). Emit-side (typechecks clean, module written) — use-count-triggered
;; spill/rematerialization reusing a shared local across widths (Bytes handle i64 vs byte/len i32).
;; OWNER: v-wasm-opt (emit/local-slot width). Graded oracles recomputed by breaker: 4/97/99/100.
;; ON FIX (v-wasm-opt lands the emit fix): rebuild cdz; gate THIS case x3 (wasm/rust/rust-async) →
;; all 4 oracles; pin into 13-strings.sexp (or the bytes/rope corpus file); baseline x3; roundtrip
;; + silent-omission + --check; MR; notify v-wasm-opt + breaker.

(case "String.to-bytes of a runtime rope reused across four branch arms compiles to a VALID module"
  (input (do
        (def (main (: mode Int64))
          (do
            (def s (String.concat "ab" (if (< mode 100) "cd" "zz")))
            (def bs (String.to-bytes s))
            (if (= mode 1) (Bytes.len bs)
                (if (= mode 2) (match (Bytes.at bs 0) ((Some b) b) ((None _u) -1))
                    (if (= mode 3) (match (Bytes.at bs 2) ((Some b) b) ((None _u) -1))
                        (match (Bytes.at bs 3) ((Some b) b) ((None _u) -1)))))))
        (export main)))
  (call main (: 1 Int64)) (output (: 4 Int64))
  (call main (: 2 Int64)) (output (: 97 Int64))
  (call main (: 3 Int64)) (output (: 99 Int64))
  (call main (: 4 Int64)) (output (: 100 Int64)))
