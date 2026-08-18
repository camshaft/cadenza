# Resume value fed to a foreign perform (2026-08-18)

- `pyf1.sexp` — (T.scale (resume s (+ s 1))): the rest-of-body value is
  handed to the OUTER handler's op as its argument at unwind time. The
  two scalings compose innermost-first (main(10): body 21 -> scale t=1:
  2*21+1=43 -> scale t=2: 2*43+2=88). The inverse dataflow of pyt1
  (there the toll's VALUE came from outside; here the resume value FLOWS
  OUT to the foreign handler). Completes the cross-handler post-resume
  matrix: value-in (pyt1), timing (pyt2), value-out (pyf1); state-arg
  (pyt3) still declines at the fold boundary. PASS x3 at a781f4674.
