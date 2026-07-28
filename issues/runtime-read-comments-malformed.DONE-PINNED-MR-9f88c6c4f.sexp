;; RIDER (breaker): malformed-read refusal ((read "(+ 1") / empty / trailing-content) currently uses
;; the DECLINE/todo channel with a permanent-sounding message ('read of text that is not a well-formed
;; s-expression over the Ast subset'). Malformedness is a PERMANENT fact about the input, not a
;; not-yet-built feature → it should be a CODED REJECT, not ride the todo channel (:5120-class
;; message-honesty, reversed direction). Since the comment-skip fix touches lower_read anyway, cheap
;; to reclassify in the same change. breaker banked a todo pin (flips to the coded reject on fix).
;; So the lower_read fix has TWO parts: (1) comment handling per the concierge ruling; (2) reclassify
;; malformed-read from decline/todo → coded reject.

;; HELD PIN (corpus-bugfix, 2026-07-28) — ruling-flagged. Origin: breaker FINDING (issue 000000017465).
;; CONFIRMED trunk 31a5f4f32: (read "(+ 1 ; c\n 2)") tokenizes the ; as a NAME node → 5-element list,
;; not (+ 1 2); (= (read commented) (quote (+ 1 2))) → 0 (silent mis-parse). Shared SexprReader
;; (rcdzc lower.rs lower_read) skips only ascii whitespace, no comment handling. Both backends.
;; RULING FLAGGED to concierge: (a) SKIP ; comments [my lean — spec self-hosting-surface.md:63 'a
;; reader MUST convert the text of A PROGRAM'; all corpus files use ; comments the front-end skips;
;; expected 11] vs (b) read takes comment-free canonical text → a ; must be REJECTED (read error),
;; NOT tokenized as a Name. Either way today is wrong. OWNER (post-ruling): rcdzc-shared lower_read
;; (v-metaprogramming / whoever owns the SexprReader). ON RULING+FIX: gate x3 → 11 (skip) or the
;; reject form (b); pin into 12-metaprogramming.sexp beside the quote/read pins; baseline x3.
;; Below is the SKIP-ruling graded case (expected 11); swap to a (declines)/error form if (b).

(case "the read primitive skips a line comment inside program text"
  (input  (do
        (def (main (: mode Int64))
          (+ (* 10 (if (= (read "(+ 1 ; a comment\n 2)") (quote (+ 1 2))) 1 0))
             (if (= (read "; leading\n(f 3)") (quote (f 3))) 1 0)))
        (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
