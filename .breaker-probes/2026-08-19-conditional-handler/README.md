# conditional-handler — the handle expression sits inside one if-branch (runtime-conditional install)
## pych1 — (if (> n 5) (handle E 3 ((tick..)) (+ (E.tick)(E.tick))) 999). Model 70/999. PASS x3.
The effect region is installed only when the guard holds; the other branch is a pure constant
(both arms type the same). Confirms a handler set up conditionally at runtime compiles + the
non-effect branch is unaffected. Promotable pass-witness.
