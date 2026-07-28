;; HELD PIN (corpus-bugfix, 2026-07-25) — do NOT land until v-runtime fixes String.concat NFC.
;; Origin: breaker FINDING (inbox issue 000000016713). CONFIRMED a genuine spec-conformance BUG on
;; trunk e8edbc737 (reproduced by corpus-bugfix): runtime String.concat does NOT NFC-normalize its
;; result, so a decomposed sequence assembled AT RUNTIME violates the String value-normalization MUSTs.
;;
;; EVIDENCE (wasm, trunk e8edbc737):
;;   • (String.concat "e" "<U+0301>")  →  byte-len 3, scalar-len 2, (= r "é") FALSE.
;;   • Spec-conformant would be: byte-len 2, scalar-len 1, equal to "é" (the composed twin).
;;   • CONTROL (passes): the reader-time decomposed literal "é" (e+U+0301 in source) DOES normalize
;;     → byte-len 2. So the reader normalizes; String.concat does not maintain the NFC invariant.
;;
;; SPEC ANALYSIS (spec/capabilities/collections-and-text.md — this is a MUST violation, NOT a parked scope):
;;   • L33-34 (MUST): "The scalar length and the byte length MUST count the string's NORMALIZED contents,
;;     so that a length is a function of the string's VALUE rather than of an incidental byte spelling
;;     that normalization removes."  → scalar-len 2 / byte-len 3 here VIOLATE this.
;;   • L53-54 (MUST): "Two strings MUST be equal exactly when their normalized contents are identical."
;;     The concat result and "é" have IDENTICAL normalized contents → they MUST be equal; they aren't.
;;   • L90-94 documents the ONLY normalization exception: a BYTE-SEQUENCE DECODE (String.from-bytes) is a
;;     faithful decode that "does not carry the Unicode composition tables," so from-bytes output "need
;;     not compare equal to a normalized literal." This exception is SCOPED TO DECODE — it does NOT
;;     mention or cover String.concat. So concat is NOT under the parked scope; it must normalize.
;;
;; ⇒ genuine runtime bug: String.concat must produce NFC-normalized output (or the runtime must maintain
;;    NFC as a String value invariant, canonicalizing at construction — breaker notes the current
;;    canonicalize-at-construction is flatten/compact, NOT NFC). OWNER: v-runtime (String rep/normalize).
;; WIDER EXPOSURE (breaker, plausible — v-runtime to confirm): Map/Set keys built from decomposed-runtime
;;    strings would miss their composed-literal twins; the symbol-interning canonical-form has the same gap.
;; ON LAND (v-runtime's concat-NFC fix on trunk): rebuild cdz; gate BOTH cases below x3 (concat→21,
;;    scalar-len→1) + the eq/byte-len faces; pin into 13-strings.sexp beside the reader-time NFC pins
;;    (:1416/:1633); baseline x3; roundtrip + silent-omission + --check; MR; notify breaker + v-runtime.

(case "a decomposed sequence assembled by runtime concat equals its composed twin (SPEC-conformance)"
  (input (do
        (def (main)
          (do
            (def r (String.concat "e" "́"))
            (+ (* (String.byte-len r) 10)
               (if (= r "é") 1 0))))
        (export main)))
  (output (: 21 Int64)))

(case "runtime concat result scalar-len counts its NORMALIZED contents"
  (input (do
        (def (main) (String.scalar-len (String.concat "e" "́")))
        (export main)))
  (output (: 1 Int64)))

;; --- COLLECTION EXPOSURES (breaker FINDING #23 widened matrix, all confirmed on trunk e8edbc737) ------
;; The same un-normalized-runtime-string bytes flow into champ_hash/champ_eq + symbol-intern, so a
;; decomposed-at-runtime key/element/symbol is UNREACHABLE by its composed spelling. One fix site
;; (normalize at concat / heap-String construction) closes ALL of these + the two String faces above.
;; 17-symbols header: symbol identity MUST be a deterministic function of CONTENT — face (2) violates it
;; directly. Each expects the spec-conformant value; actual today is the miss (shown in comments).

(case "a map key built by runtime concat is reachable by its composed spelling"
  (doc    "Map-key face: insert under (String.concat \"e\" \"<U+0301>\"), look up with the composed
           \"é\" — normalized contents identical, so the lookup MUST hit (7). Today MISSES → None (-1):
           champ_hash/champ_eq key on the un-normalized physical bytes. Closed by normalize-at-construction.")
  (input (do
        (def (main)
          (match (Map.lookup (Map.insert (Map.empty) (String.concat "e" "́") 7) "é")
            ((Some v) v) ((None) (- 0 1))))
        (export main)))
  (output (: 7 Int64)))

(case "a symbol interned from a runtime concat equals its composed symbol literal"
  (doc    "Symbol face: (Symbol.of (String.concat \"e\" \"<U+0301>\")) MUST equal #\"é\" — symbol identity
           is a deterministic function of CONTENT (17-symbols; symbol-interning canonical form). Today
           UNEQUAL (0): the intern keys on un-normalized bytes. Closed by the same normalize-at-construction.")
  (input (do
        (def (main) (if (= (Symbol.of (String.concat "e" "́")) #"é") 1 0))
        (export main)))
  (output (: 1 Int64)))

(case "set membership of a composed string over a decomposed-runtime element"
  (doc    "Set face: a set holding (String.concat \"e\" \"<U+0301>\") MUST contain the composed \"é\"
           (identical normalized contents). Today FALSE (0): Set membership keys on un-normalized bytes,
           same champ path. Closed by normalize-at-construction.")
  (input (do
        (def (main) (if (Set.contains (Set.of (list (String.concat "e" "́"))) "é") 1 0))
        (export main)))
  (output (: 1 Int64)))
