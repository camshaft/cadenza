; BREAKER FINDING — capability gap (honest DECLINE, not a miscompile): `Symbol.of` cannot intern a
; GENUINELY-RUNTIME string — only constant strings. The runtime-intern analogue of the runtime-String.slice
; gap I filed earlier (which was since fixed, 38df2b9b0).
;
; `(Symbol.of s)` with `s` arriving at the call boundary (a runtime String value) DECLINES:
;   cdz: error: Symbol.of on a runtime string is not yet interned (constant strings only)
; A CONSTANT-string `(Symbol.of "abc")` works, and a runtime Symbol VALUE (a Symbol param flowing through a
; function, as in 17-symbols.sexp cases 125/136/151) works too — the gap is specifically INTERNING a runtime
; STRING into a Symbol.
;
; NOT a mislabeling issue (unlike the old slice cases): the existing "runtime symbol" cases legitimately test
; a runtime Symbol VALUE (resolve takes a Symbol param), which is a different, working path. This gap is only
; the runtime-string→Symbol intern.
;
; VERIFIED on trunk: `(Symbol.of s)` [call-boundary s] declines; `(Symbol.of "abc")` [constant] works.
;
; SUGGESTED (v-runtime / symbols owner): implement runtime Symbol interning — a runtime `str-intern` op that
; hashes the runtime string bytes into the symbol table at run time, the intern analogue of the runtime
; String.slice byte-walk that landed. Until then this is a clean decline (reject-don't-miscompile), so it is
; LOW priority — filing so the "constant strings only" limit is tracked, not lost. The case below is graded
; `declines` (passes today as a decline; flips to a value + should become `output "abc"` when interning lands).

(case "adv symbol: Symbol.of on a genuinely-runtime string declines (constant-only interning)"
  (doc "`Symbol.of` on a String arriving at main's call boundary cannot intern at compile time and DECLINES
        ('Symbol.of on a runtime string is not yet interned (constant strings only)'). A constant-string
        Symbol.of works, and a runtime Symbol VALUE works; only interning a runtime STRING is unsupported.
        Honest reject-don't-miscompile; flips to output when a runtime str-intern op lands (the intern
        analogue of the runtime String.slice byte-walk).")
  (input (do (def (main (: s String)) (Symbol.to-string (Symbol.of s))) (export main)))
  (call main (: "abc" String))
  (declines))

(case "adv symbol: Symbol.of on a constant string works (the boundary control)"
  (doc "The control that PASSES: a CONSTANT-string Symbol.of interns at compile time and round-trips.
        Pins that the decline above is specifically the RUNTIME-string intern path, not Symbol.of in
        general — the constant path is fully supported.")
  (input (do (def (main) (Symbol.to-string (Symbol.of "abc"))) (export main)))
  (output (: "abc" String)))
