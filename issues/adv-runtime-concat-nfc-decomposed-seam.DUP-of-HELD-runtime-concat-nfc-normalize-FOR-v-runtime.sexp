; FINDING (breaker, 2026-07-25): runtime String.concat does NOT NFC-normalize its result,
; so a decomposed sequence assembled AT RUNTIME compares UNEQUAL to its composed twin —
; while collections-and-text.md #String Equality Follows Normalized Contents says two strings
; "MUST be equal exactly when their normalized contents are identical."
;
; EVIDENCE (wasm, trunk e8edbc737):
;   READER-time: a decomposed literal "e+U+0301" IS normalized — byte-len 2, (= dec comp) true → 21
;     (matches the landed 13-strings :1416/:1633 source-spelling pins)
;   RUNTIME: (String.concat "e" "<U+0301>") → byte-len 3 (NOT 2), (= r "é") FALSE → 30,
;     scalar-len 2 (NOT 1) — the combining char stays a separate scalar.
;
; The normalized contents of the concat result and the composed literal are IDENTICAL ("é"),
; so per the spec MUST they should be equal; physically they differ (no NFC at the concat seam).
; CAVEAT: this may be an intentionally-parked design scope ("under the text normalization the
; hashing-and-encoding choice pins" — if that choice scopes normalization to CONSTRUCTION-from-
; source only, the spec text should say so). Filing as a spec-conformance question for routing:
; either concat normalizes (runtime fix), or the spec/corpus pins the narrower contract.
;
; Repro (expect 21 if concat normalizes per spec; actual today: 30):
(case "a decomposed sequence assembled by runtime concat equals its composed twin (SPEC-conformance repro)"
  (input (do
        (def (main)
          (do
            (def r (String.concat "e" "́"))
            (+ (* (String.byte-len r) 10)
               (if (= r "é") 1 0))))
        (export main)))
  (output (: 21 Int64)))
