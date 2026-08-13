# 2026-08-13 modular inverse via extended Euclid (tick 1442)

- `inv1.sexp` — the arm runs iterative extended Euclid (4-param recursion
  threading both remainder and Bezout-coefficient pairs), normalizes the
  possibly-NEGATIVE coefficient through the double-mod idiom, and the BODY
  VERIFIES the algebra: (n * extracted-inverse) % 97 == 1 rides as the final
  packed digit. Value-dependent iteration depth; negative intermediates in the
  Bezout thread (eg 5,97 walks s through 0,1,-19,20,-39... normalized 39).
  Composes rot1's norm idiom + fra1's gcd-as-subroutine + a body-side
  correctness proof. PASS ×3 (39106521/68106521).
