; FINDING (breaker, 2026-07-28): the runtime `read` primitive does NOT skip `;` line comments —
; it tokenizes the semicolon as a NAME. `(read "(+ 1 ; c\n 2)")` parses to the 5-element list
; (+ 1 |;| c 2) — VERIFIED equal to (Ast.List [Name "+", Int 1, Name ";", Name "c", Int 2]) —
; instead of (+ 1 2). Both backends identical (shared lower_read/SexprReader whose skip_ws
; handles only ascii whitespace; lower.rs:2947-area).
;
; Why it matters: self-hosting-surface.md "A Reader Converts Text To The Canonical
; Representation" — the text of a PROGRAM includes comments (every corpus file uses `;`; the
; compiler's own front-end reader skips them). A guest tool that reads real program text (the
; self-hosting use case this file exists for) silently mis-parses any commented source: no
; error, a WRONG AST with |;| name nodes. Silent-wrong-value class, not a decline.
;
; (If instead the ruling is "the read PRIMITIVE takes comment-free canonical text only", the
; right behavior is an explicit REJECT of `;` — never a silent Name token. Either ruling makes
; today's behavior wrong.)
;
; GRADED REPRO (= fix pin under the skip-comments ruling; FAILS today, both rows 0):
(case "the read primitive skips a line comment inside program text"
  (input  (do
        (def (main (: mode Int64))
          (+ (* 10 (if (= (read "(+ 1 ; a comment\n 2)") (quote (+ 1 2))) 1 0))
             (if (= (read "; leading\n(f 3)") (quote (f 3))) 1 0)))
        (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
