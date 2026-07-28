;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-wasm-opt fixes the invalid-module emit
;; on the recursive-String-param-spill path. Origin: breaker FINDING (issue 000000017338). CONFIRMED
;; trunk 600865d68: [main 0] expected 3, wasm trapped 'invalid component: failed to compile:
;; wasm[0]::function[13]' (rust PASSES 3 — the oracle). A recursive fn rebinding its String param to a
;; helper concat-of-two-slices emits INVALID wasm (i64-where-i32, func validate) when the helper `d`
;; slices by its own Int64 index param AND the recursion exit reads String.scalar-len of the REBOUND
;; param. DISCRIMINATOR: BOTH ingredients required (either alone clean — breaker matrix m9/m10). Same
;; wasm-tools signature as the FIXED to-bytes br_table bug 4f9658803, but a DIFFERENT seam: the
;; recursive-param SPILL path. OWNER: v-wasm-opt. rust=3 oracle. Real-world: string-shrinker shape
;; (property-testing users write exactly this). ON FIX: rebuild cdz; gate x3 → 3; pin into
;; 13-strings.sexp (or 10-bytes); baseline x3; roundtrip + silent-omission + --check; MR; notify
;; v-wasm-opt + breaker.

(case "a recursive drop-scalar walk over a rope converges (string-shrinker shape)"
  (input  (do
        (def (d (: s String) (: i Int64))
          (String.concat (Option.expect (String.slice s 0 i) "lo")
                         (Option.expect (String.slice s (+ i 1) (String.scalar-len s)) "hi")))
        (def (walk (: s String) (: i Int64))
          (if (>= i (String.scalar-len s))
              s
              (walk (d s i) (+ i 1))))
        (def (main (: mode Int64))
          (String.byte-len (walk "aébcd" 0)))
        (export main)))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
