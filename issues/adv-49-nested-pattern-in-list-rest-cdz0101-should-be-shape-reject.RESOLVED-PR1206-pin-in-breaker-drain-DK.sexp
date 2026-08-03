; adv-49 (breaker tick 1008) — DIAGNOSTICS-QUALITY: a NESTED pattern in list-REST position rejects
; with CDZ0101 'unbound name' — the WRONG diagnostic for what is a SHAPE error.
;
; RULING (v-inference via concierge, 2026-08-02): name/wildcard-only rest is INTENDED —
; core-semantics.md:149 grants nested patterns only to ELEMENT positions; the rest binder must be
; irrefutable (:135 'A Binding Position Accepts An Irrefutable Pattern'; a nested list pattern is
; refutable on empty rest). Same rule as map-rest name-only (:161-163). So the REJECT is correct,
; but resolve currently treats the rest slot as a single name-binder and lets the compound's inner
; names (b, r below) fall through to scoping → CDZ0101 'unbound name', a misleading message for a
; user who wrote a plausible-but-invalid shape.
;
; WANTED: a SHAPE reject naming the rest form — e.g. "the rest binder of a list pattern must be a
; name or wildcard, not a nested pattern; bind the tail to a name and destructure it in a nested
; match" — mirroring the map-rest shape reject pinned at 05-compound-types.sexp:15967 (which names
; the malformed-rest shape rather than leaking an unbound-binder error).
;
; Observed: CDZ0101 on wasm + rust + rust-async (consistent). The (error CDZ0101) row below grades
; PASS against TODAY'S behavior — when v-diagnostics lands the shape diagnostic this file's expected
; code flips to the new one, and the corpus pin (corpus-bugfix lane) should be added THEN with the
; corrected code, not before (else the pin breaks on the diag fix).
;
; Related but SEPARATE (defensible as-is, no action): the trailing-element form
; (list .. init last) rejects CDZ0201 — a distinct shape rule, not part of this filing.

(case "a nested list pattern in rest position is a shape error, not an unbound name"
  (doc    "RULED (v-inference 2026-08-02): the rest binder of a list pattern admits only a name or
           wildcard (core-semantics.md:149 grants nested patterns to ELEMENT positions only; :135
           requires a binding position to hold an irrefutable pattern, and a nested list pattern is
           refutable on empty rest — the same name-only rule the map rest binder has). `(list a ..
           (list b .. r))` must therefore REJECT — but with a SHAPE message naming the rest form,
           not the current CDZ0101 'unbound name' scoping leak this case RECORDED post-fix (PR#1206 landed the shape reject; #1250 aligned wording) — was CDZ0101 pre-fix, now the
           v-diagnostics fix.")
  (input  (do
            (def (main (: xs (List Int64)))
              (match xs
                ((list a .. (list b .. r)) (+ (* 100 a) (+ (* 10 b) (List.len r))))
                ((list a) (* a 1000))
                ((list) -1)))
            (export main)))
  (call   main (list 1 2 3))
  (error  CDZ0201))
