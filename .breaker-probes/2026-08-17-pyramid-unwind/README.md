# Pyramid unwind, post-resume arithmetic (2026-08-17)

- `pyr1.sexp` — POST-RESUME ARITHMETIC in the arm: (+ (resume s (+ s 2))
  (* 1000 (+ s 1))). Sweep across ALL 1481 resumes in 14c: zero cases do
  arithmetic on resume's return value ((+ (resume / (* (resume / (let ((x
  (resume / (if (resume / (match (resume all count 0) — every existing arm
  is resume-in-tail-position. Deep-handler semantics: resume returns the
  rest-of-body's final value, so three dispatches stack three +1000*(s+1)
  tolls that unwind INNERMOST-FIRST after the body's positional fold; each
  toll is keyed to the state AT ITS OWN DISPATCH (s captured pre-resume),
  so a reordered unwind or a toll reading post-resume state misprices the
  total. Hand-model pins n=10 -> 12531 (fold 531 + tolls 3000+5000+4000? no:
  tolls 5000+3000+... see model: body 531, +1000*(s2+1)=5000, +1000*(s1+1)
  =3000... final 12531) and n=0 -> 9420. PASS x3 at dc649b874.
