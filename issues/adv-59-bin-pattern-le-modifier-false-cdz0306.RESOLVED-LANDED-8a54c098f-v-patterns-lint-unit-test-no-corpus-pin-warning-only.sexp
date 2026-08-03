; adv-59 (breaker, 2026-08-02, LOW-MED diagnostics — semantics CORRECT, lint FALSE + its advice BREAKS):
; the `le` byte-order modifier in a bin PATTERN segment `(bin (u16 n le))` is HONORED semantically
; (reads little-endian, n=258 from bytes [2,1] — the corpus pins this shape at 16-binary:165) but the
; unused-binding lint misclassifies the modifier as a match BINDER:
;   observed 1: every le-modifier pattern emits `warning [CDZ0306] unused match binding: `le` is
;               never used (prefix with `_` to silence)` — FALSE (le is a modifier, not a binding;
;               there is nothing to "use").
;   observed 2: FOLLOWING the warning's advice — `(bin (u16 n _le))` — is a hard ERROR pair:
;               `[CDZ0201] the only integer bin-segment modifier is `le`` + `[CDZ0101] unbound name n`
;               (the whole segment stops parsing as a modifier form). So the lint's suggested fix
;               breaks a working program.
; expected: no CDZ0306 for a segment MODIFIER (the pattern-side `le` should be excluded from the
;           unused-binding walk, as the construction-side `le` already is — `(bin (u16 258 le))`
;           emits no warning).
; faces:    u16/u32/i16 pattern segments all warn identically; construction-side le never warns.
; note:     this warning fires on the CORPUS's own pinned shape (16-binary-matching.sexp:165) — the
;           gate ignores warnings so it stays green, but any user compiling that exact pinned idiom
;           gets told to make a breaking edit.
(case "adv-59 the le modifier in a bin pattern is not an unused binding (no CDZ0306; its advice must not break)"
  (input  (do
            (def (main)
              (match (Bytes.of (list 2 1))
                ((bin (u16 n le)) (Int64.of n))
                (_ -1)))
            (export main)))
  (call   main) (output (: 258 Int64)))
