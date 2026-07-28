;; HELD PIN (corpus-bugfix, 2026-07-27) — do NOT land until v-inference fixes recursive-result
;; inference for a mixed BigInt/Int64 fn whose recursive-call result is locally BOUND.
;; Origin: breaker FINDING (inbox issue 000000017025). CONFIRMED reproduces on trunk ae1054ca6
;; (fresh build): the do-def + result-used-ONCE spelling declines with a BOGUS
;;   CDZ0201 "member access requires a record, found Type"  AT the recursive call site (bi:4:57).
;; f is a function, not a Type — the recursive fn reference is mis-resolved to a Type during
;; recursive-RESULT type flow. Sibling spellings (breaker matrix): do-def+used-TWICE = compiler
;; HANG >100s CPU (DO NOT gate — hangs the harness); let-form = bogus CDZ0301 Int64-vs-BigInt.
;; WORKS: tail-position unbound recursion; non-recursive same shape; scalar-only recursive do-def.
;; IMPACT: recursive modpow/repeated-squaring unwritable (the 06-corpus modpow pin is Int64-only).
;; ON FIX (v-inference lands recursive-result inference fix): rebuild cdz; gate THIS case x3
;; (wasm/rust/rust-async) → 7; pin into the appropriate numeric/recursion corpus file beside the
;; existing modpow/BigInt pins; baseline x3; roundtrip + silent-omission + --check; MR; notify
;; v-inference + breaker. Gate ONLY this CDZ0201 spelling — NEVER the used-twice HANG spelling.

(case "a recursive BigInt fn whose bound result feeds a mod compiles and computes (FINDING)"
  (input (do
        (def (f (: base BigInt) (: e Int64) (: md BigInt))
          (if (= e 0)
              base
              (do
                (def hh (f base (/ e 2) md))
                (% hh md))))
        (def (main (: e Int64))
          (Int64.of (f (BigInt.of 7) e (BigInt.of 100))))
        (export main)))
  (call main (: 8 Int64)) (output (: 7 Int64)))
